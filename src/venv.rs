use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use std::ffi::CString;

use fancy_regex::Regex;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use pyo3::exceptions::PySystemExit;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyAnyMethods, PyDict, PyIterator, PyString};
use sha2::{Digest, Sha256};
use shell_words::split;

use crate::config::Selector;
use crate::constants::VENV_PREFIX;

#[derive(Clone)]
#[pyclass(name = "Venv", module = "riot")]
pub struct PyVenv {
    pub name: Option<String>,
    pub command: Option<String>,
    pub pys: Vec<String>,
    pkgs: IndexMap<String, Vec<String>>,
    env: IndexMap<String, Vec<String>>,
    pub create: Option<bool>,
    pub skip_dev_install: Option<bool>,
    pub venvs: Vec<Self>,
}

#[pymethods]
impl PyVenv {
    #[new]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(
        signature = (name=None, command=None, pys=None, pkgs=None, env=None, venvs=None, create=None, skip_dev_install=None)
    )]
    fn new(
        py: Python<'_>,
        name: Option<String>,
        command: Option<String>,
        pys: Option<Py<PyAny>>,
        pkgs: Option<Py<PyAny>>,
        env: Option<Py<PyAny>>,
        venvs: Option<Py<PyAny>>,
        create: Option<bool>,
        skip_dev_install: Option<bool>,
    ) -> PyResult<Self> {
        let venvs = venvs
            .map(|value| value.bind(py).extract::<Vec<Self>>())
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            name,
            command,
            pys: parse_pys(py, pys)?,
            pkgs: parse_dict_to_vec_map(py, pkgs)?,
            env: parse_dict_to_vec_map(py, env)?,
            create,
            skip_dev_install,
            venvs,
        })
    }
}

/// Leaf configuration after all inheritance has been applied.
#[derive(Clone)]
pub struct RiotVenv {
    pub name: String,
    pub python: String,
    pub pkgs: IndexMap<String, String>,
    pub hash: String,
    pub services: Vec<String>,
    pub execution_contexts: Vec<ExecutionContext>,
    pub shared_pkgs: IndexMap<String, String>,
    pub shared_env: IndexMap<String, String>,
}

impl RiotVenv {
    fn new(
        name: String,
        python: String,
        pkgs: IndexMap<String, String>,
        hash: String,
        services: Vec<String>,
    ) -> Self {
        Self {
            name,
            python,
            pkgs,
            hash,
            services,
            execution_contexts: Vec::new(),
            shared_pkgs: IndexMap::new(),
            shared_env: IndexMap::new(),
        }
    }
}

/// Resolved execution context for a virtual environment variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    pub command: Option<String>,
    pub pytest_target: Option<String>,
    pub env: IndexMap<String, String>,
    pub create: bool,
    pub skip_dev_install: bool,
    pub hash: String,
}

/// Compute the virtual environment path from the riot root and short hash.
/// The '@' character in the `short_hash` is replaced with '_' for filesystem compatibility.
pub fn venv_path(riot_root: &Path, short_hash: &str) -> PathBuf {
    riot_root.join(format!("{}{}", VENV_PREFIX, short_hash.replace('@', "_")))
}

/// Compute the python executable path for a virtual environment.
pub fn venv_python_path(riot_root: &Path, short_hash: &str) -> String {
    venv_path(riot_root, short_hash)
        .join("bin/python")
        .to_string_lossy()
        .to_string()
}

/// State carried while traversing a virtualenv tree.
#[derive(Clone, Debug, Default)]
struct ResolvedSpec {
    name: Option<String>,
    command: Option<String>,
    pys: Option<Vec<String>>,
    pkgs: IndexMap<String, Vec<String>>,
    env: IndexMap<String, Vec<String>>,
    create: bool,
    skip_dev_install: bool,
}

impl ResolvedSpec {
    fn merge(&self, venv: &PyVenv) -> Option<Self> {
        let mut next = self.clone();

        if let Some(name) = &venv.name {
            next.name = Some(name.clone());
        }

        if let Some(command) = &venv.command {
            next.command = Some(command.clone());
        }

        if let Some(create) = venv.create {
            next.create = create;
        }

        if let Some(skip) = venv.skip_dev_install {
            next.skip_dev_install = skip;
        }

        for (pkg, values) in &venv.pkgs {
            if !values.is_empty() {
                next.pkgs.insert(pkg.clone(), values.clone());
            }
        }

        for (key, values) in &venv.env {
            if !values.is_empty() {
                next.env.insert(key.clone(), values.clone());
            }
        }

        let mut pys = next.pys.take();
        if !venv.pys.is_empty() {
            if let Some(parent_pys) = &self.pys {
                let compatible = venv.pys.iter().any(|candidate| {
                    parent_pys
                        .iter()
                        .any(|parent_py| python_versions_compatible(parent_py, candidate))
                });
                if !compatible {
                    return None;
                }
            }
            pys = Some(venv.pys.clone());
        } else if let Some(parent_pys) = &self.pys {
            pys = Some(parent_pys.clone());
        }

        if let Some(values) = pys.as_ref() {
            if values.is_empty() {
                return None;
            }
        }
        next.pys = pys;

        Some(next)
    }
}

impl ExecutionContext {
    fn new(
        command: Option<String>,
        env: IndexMap<String, String>,
        create: bool,
        skip_dev_install: bool,
        base_hash: &str,
        ctx_hash: &str,
    ) -> Self {
        let pytest_target = command.as_deref().and_then(parse_pytest_target);
        Self {
            command,
            pytest_target,
            env,
            create,
            skip_dev_install,
            hash: format!("{base_hash}@{ctx_hash}"),
        }
    }
}

/// Expand every leaf of a virtualenv tree into a concrete configuration grouped by base hash.
fn normalize_venvs(
    py: Python<'_>,
    root: &PyVenv,
    project_path: &Path,
) -> IndexMap<String, RiotVenv> {
    let mut venvs = IndexMap::new();
    let service_map = get_services(py, project_path);
    collect_riot_venvs(
        py,
        root,
        &ResolvedSpec::default(),
        &mut venvs,
        service_map.as_ref(),
    );
    for venv in venvs.values_mut() {
        venv.shared_env = shared_entries(venv.execution_contexts.iter().map(|ctx| &ctx.env));
    }
    venvs
}

fn get_services(py: Python<'_>, project_path: &Path) -> Option<HashMap<String, Vec<String>>> {
    let python_code = format!(
        r#"
import sys
sys.path.insert(0, r"{}")

from tests.suitespec import SUITESPEC
result = {{ k.split("::")[-1]: (v.get("services", []) + ["testagent"] if v.get("snapshot") else []) for k, v in SUITESPEC["suites"].items() if v.get("services") or v.get("snapshot") }}
"#,
        project_path.to_string_lossy()
    );

    let code_cstr = CString::new(python_code).ok()?;
    let module_name = c"_get_service";
    let filename = c"get_services";

    let result: Result<HashMap<String, Vec<String>>, PyErr> =
        PyModule::from_code(py, code_cstr.as_c_str(), filename, module_name)
            .and_then(|module| module.getattr("result")?.extract());

    result.ok()
}

fn collect_riot_venvs(
    py: Python,
    venv: &PyVenv,
    state: &ResolvedSpec,
    acc: &mut IndexMap<String, RiotVenv>,
    service_map: Option<&HashMap<String, Vec<String>>>,
) {
    let Some(next_state) = state.merge(venv) else {
        return;
    };

    if venv.venvs.is_empty() {
        if let (Some(name), Some(pys)) = (&next_state.name, &next_state.pys) {
            let pkg_variants = expand_product(&next_state.pkgs);
            let env_variants = expand_product(&next_state.env);
            if pkg_variants.is_empty() || env_variants.is_empty() {
                return;
            }

            for py_version in pys {
                let interpreter_repr = interpreter_repr(py, py_version);
                for pkgs in &pkg_variants {
                    let full_pkg_str = pip_deps(pkgs);
                    let name_repr = python_repr_str(py, name);
                    let hash =
                        RiotHasher::hash_parts(&[&name_repr, &interpreter_repr, &full_pkg_str]);

                    let services = service_map.map_or_else(Vec::new, |service_map| {
                        service_map.get(name).cloned().unwrap_or_default()
                    });
                    let entry = acc.entry(hash.clone()).or_insert_with(|| {
                        RiotVenv::new(
                            name.clone(),
                            py_version.clone(),
                            pkgs.clone(),
                            hash.clone(),
                            services,
                        )
                    });

                    let command = next_state.command.clone();
                    let base_hash = entry.hash.clone();
                    for env in &env_variants {
                        let context_env = env.clone();
                        let ctx_hash = RiotHasher::context_hash(
                            py,
                            command.as_ref(),
                            &context_env,
                            next_state.create,
                            next_state.skip_dev_install,
                        );

                        let full_hash = format!("{base_hash}@{ctx_hash}");
                        if entry
                            .execution_contexts
                            .iter()
                            .any(|ctx| ctx.hash == full_hash)
                        {
                            continue;
                        }

                        entry.execution_contexts.push(ExecutionContext::new(
                            command.clone(),
                            context_env,
                            next_state.create,
                            next_state.skip_dev_install,
                            &base_hash,
                            &ctx_hash,
                        ));
                    }
                }
            }
        }
        return;
    }

    for child in &venv.venvs {
        collect_riot_venvs(py, child, &next_state, acc, service_map);
    }
}

fn expand_product(values: &IndexMap<String, Vec<String>>) -> Vec<IndexMap<String, String>> {
    if values.values().any(std::vec::Vec::is_empty) {
        return Vec::new();
    }

    values
        .iter()
        .map(|(key, entries)| entries.iter().map(|entry| (key.clone(), entry.clone())))
        .multi_cartesian_product()
        .map(|pairs| pairs.into_iter().collect())
        .collect()
}

fn shared_entries<'a, I>(maps: I) -> IndexMap<String, String>
where
    I: IntoIterator<Item = &'a IndexMap<String, String>>,
{
    let mut iter = maps.into_iter();
    let Some(first) = iter.next() else {
        return IndexMap::new();
    };
    let mut shared = first.clone();
    for map in iter {
        shared.retain(|key, val| map.get(key).is_some_and(|other| other == val));
    }
    shared
}

/// Reproduce riot's quoted pip dependency formatting.
fn pip_deps(pkgs: &IndexMap<String, String>) -> String {
    let mut parts = Vec::with_capacity(pkgs.len());
    for (lib, version) in pkgs {
        parts.push(format!("'{lib}{version}'"));
    }
    parts.join(" ")
}

fn dedup_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = IndexSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            if seen.insert(value.clone()) {
                Some(value)
            } else {
                None
            }
        })
        .collect()
}

fn normalize_pys(mut versions: Vec<String>) -> Vec<String> {
    versions.retain(|value| !value.is_empty());
    versions.sort_by(|a, b| compare_python_versions(a, b));
    versions.dedup();
    versions
}

fn extract_str_list(obj: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    if obj.is_none() {
        return Ok(Vec::new());
    }

    if let Ok(value) = obj.cast::<PyString>() {
        return Ok(vec![value.extract::<String>()?]);
    }

    if let Ok(iter) = PyIterator::from_object(obj.as_any()) {
        let mut values = Vec::new();
        for item in iter {
            let item: Bound<'_, PyAny> = item?;
            if item.is_none() {
                continue;
            }
            values.push(item.extract::<String>()?);
        }
        return Ok(values);
    }

    obj.extract().map(|value| vec![value])
}

fn parse_version_components(version: &str) -> Option<Vec<u32>> {
    if version.is_empty() {
        return Some(vec![]);
    }

    let mut components = Vec::new();
    for part in version.split('.') {
        let parsed = part.parse::<u32>().ok()?;
        components.push(parsed);
    }
    Some(components)
}

pub fn compare_python_versions(lhs: &str, rhs: &str) -> Ordering {
    match (parse_version_components(lhs), parse_version_components(rhs)) {
        (Some(mut left), Some(mut right)) => {
            let max_len = left.len().max(right.len());
            left.resize(max_len, 0);
            right.resize(max_len, 0);
            for (l, r) in left.iter().zip(right.iter()) {
                let ord = l.cmp(r);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        }
        _ => lhs.cmp(rhs),
    }
}

/// Return true when two python selectors can overlap (prefix matching on dotted numbers).
fn python_versions_compatible(parent: &str, child: &str) -> bool {
    if parent.is_empty() || child.is_empty() {
        return true;
    }

    if parent == child {
        return true;
    }

    match (
        parse_version_components(parent),
        parse_version_components(child),
    ) {
        (Some(parent_components), Some(child_components)) => {
            let len = parent_components.len().min(child_components.len());
            parent_components[..len] == child_components[..len]
        }
        _ => parent.starts_with(child) || child.starts_with(parent),
    }
}

fn python_repr_str(py: Python<'_>, value: &str) -> String {
    let py_str = PyString::new(py, value);
    py_str.repr().map_or_else(
        |_| format!("{value:?}"),
        |repr_obj| repr_obj.extract().unwrap_or_else(|_| format!("{value:?}")),
    )
}

fn interpreter_repr(py: Python<'_>, py_hint: &str) -> String {
    format!("Interpreter(_hint={})", python_repr_str(py, py_hint))
}

/// Extract long and short hash from Python hex string (strips '0x' prefix and takes first 7 chars).
fn extract_hash(hex_str: &str) -> String {
    let long_hash = hex_str.chars().skip(2).collect::<String>();
    long_hash.chars().take(7).collect()
}

struct RiotHasher;

impl RiotHasher {
    const HASH_MODULUS_64: u128 = (1u128 << 61) - 1;
    const HASH_MODULUS_32: u128 = (1u128 << 31) - 1;

    fn hash_parts(parts: &[&str]) -> String {
        let mut sha = Sha256::new();
        for part in parts {
            sha.update(part.as_bytes());
        }

        let digest = sha.finalize();
        let modulus = if cfg!(target_pointer_width = "64") {
            Self::HASH_MODULUS_64
        } else {
            Self::HASH_MODULUS_32
        };

        let mut remainder: u128 = 0;
        for byte in digest {
            remainder = ((remainder << 8) + u128::from(byte)) % modulus;
        }

        // Positive digest, so no sign adjustment needed.
        let mut hash_value = remainder.cast_signed();
        if hash_value == -1 {
            hash_value = -2;
        }

        let hex_str = format!("{hash_value:#x}");
        extract_hash(&hex_str)
    }

    fn context_hash(
        py: Python<'_>,
        command: Option<&String>,
        env: &IndexMap<String, String>,
        create: bool,
        skip_dev_install: bool,
    ) -> String {
        let command_repr =
            command.map_or_else(|| "None".to_string(), |value| python_repr_str(py, value));

        let env_repr = if env.is_empty() {
            String::new()
        } else {
            env.iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("|")
        };

        let create_flag = if create { "true" } else { "false" };
        let skip_flag = if skip_dev_install { "true" } else { "false" };

        Self::hash_parts(&[&command_repr, &env_repr, create_flag, skip_flag])
    }
}

fn parse_pytest_target(command: &str) -> Option<String> {
    let tokens = split(command).ok()?;

    let pytest_idx = tokens.iter().position(|token| token == "pytest")?;

    for token in tokens.iter().skip(pytest_idx + 1) {
        if token.starts_with('-') || token.contains('{') || Path::new(token).is_absolute() {
            continue;
        }

        let candidate = PathBuf::from(token);

        if (candidate.is_dir() || candidate.extension().is_some_and(|ext| ext == "py"))
            && candidate.exists()
        {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    None
}

fn missing_venv_err() -> PyErr {
    eprintln!("error: riotfile does not define a `venv` variable");
    PyErr::new::<PySystemExit, _>(1)
}

fn load_riotfile(py: Python<'_>, path: &Path) -> PyResult<PyVenv> {
    let source = fs::read_to_string(path).map_err(|err| {
        eprintln!("error: failed to read riotfile: {err}");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let source_cstr = CString::new(source).map_err(|err| {
        eprintln!("error: invalid riotfile content: {err}");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let module_name = CString::new(
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("riotfile"),
    )
    .map_err(|err| {
        eprintln!("error: invalid module name: {err}");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let path_cstr = CString::new(path.to_string_lossy().into_owned()).map_err(|err| {
        eprintln!("error: invalid riotfile path: {err}");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let module = PyModule::from_code(
        py,
        source_cstr.as_c_str(),
        path_cstr.as_c_str(),
        module_name.as_c_str(),
    )?;

    let venv_obj = module.getattr("venv").map_err(|_| missing_venv_err())?;
    if venv_obj.is_none() {
        return Err(missing_venv_err());
    }

    venv_obj.extract::<PyVenv>().map_err(PyErr::from)
}

/// Accept any of riot's `pys` shorthands (scalar, list, tuple, iterable) and normalise to strings.
fn parse_pys(py: Python<'_>, pys: Option<Py<PyAny>>) -> PyResult<Vec<String>> {
    let versions = pys
        .map(|obj| extract_str_list(obj.bind(py)))
        .transpose()?
        .unwrap_or_default();
    Ok(normalize_pys(versions))
}

/// Parse a Python dictionary into an `IndexMap` of string keys to vector of string values.
/// Accepts dict values as scalars, lists, or tuples and normalizes them to vectors.
fn parse_dict_to_vec_map(
    py: Python<'_>,
    obj: Option<Py<PyAny>>,
) -> PyResult<IndexMap<String, Vec<String>>> {
    let mut map = IndexMap::new();
    let Some(obj) = obj else {
        return Ok(map);
    };

    let bound = obj.bind(py);
    if bound.is_none() {
        return Ok(map);
    }

    let dict = bound.cast::<PyDict>()?;
    for (key, value) in dict {
        if value.is_none() {
            continue;
        }
        let name: String = key.extract()?;
        let values = dedup_preserving_order(extract_str_list(value.as_any())?);
        map.insert(name, values);
    }

    Ok(map)
}

fn is_short_hash(ident: &str) -> bool {
    ident.len() == 7 && ident.chars().all(|c| char::is_ascii_hexdigit(&c))
}

fn parse_ctx_hash(ident: &str) -> Option<&str> {
    let mut split = ident.split('@');
    let venv_hash = split.next()?;
    let exc_hash = split.next()?;
    if split.next().is_none() && is_short_hash(venv_hash) && is_short_hash(exc_hash) {
        return Some(venv_hash);
    }
    None
}

fn shared_pkgs_by_name<'a, I>(venvs: I) -> HashMap<String, IndexMap<String, String>>
where
    I: IntoIterator<Item = &'a RiotVenv>,
{
    let mut grouped: HashMap<String, Vec<&'a IndexMap<String, String>>> = HashMap::new();
    for venv in venvs {
        grouped
            .entry(venv.name.clone())
            .or_default()
            .push(&venv.pkgs);
    }

    grouped
        .into_iter()
        .map(|(name, pkgs)| (name, shared_entries(pkgs)))
        .collect()
}

pub fn select_execution_contexts(
    py: Python<'_>,
    riotfile_path: &Path,
    selector: Selector,
) -> PyResult<Vec<RiotVenv>> {
    let root = load_riotfile(py, riotfile_path)?;
    let project_path = riotfile_path.parent().unwrap();
    let mut riot_venvs = normalize_venvs(py, &root, project_path);

    let (pattern_selector, python_selector) = match selector {
        Selector::All => (String::new(), None),
        Selector::Pattern(pattern) => (pattern, None),
        Selector::Generic { python, pattern } => (pattern.unwrap_or_default(), python),
    };

    if let Some(python_selector) = python_selector {
        riot_venvs.retain(|_, venv| python_selector.contains(&venv.python));
    }
    let shared_pkgs_map = shared_pkgs_by_name(riot_venvs.values());

    if is_short_hash(&pattern_selector) {
        let Some(mut venv) = riot_venvs.get(&pattern_selector).cloned() else {
            return Ok(vec![]);
        };
        venv.shared_pkgs = shared_pkgs_map.get(&venv.name).cloned().unwrap_or_default();
        return Ok(vec![venv]);
    }

    if let Some(venv_hash) = parse_ctx_hash(&pattern_selector) {
        let Some(mut venv) = riot_venvs.get(venv_hash).cloned() else {
            return Ok(vec![]);
        };

        let shared_env = venv.shared_env.clone();
        venv.shared_pkgs = shared_pkgs_map.get(&venv.name).cloned().unwrap_or_default();
        venv.execution_contexts
            .retain(|ctx| ctx.hash == pattern_selector);
        venv.shared_env = shared_env;
        return Ok(vec![venv]);
    }

    let name_regex = Regex::new(&pattern_selector).map_err(|err| {
        eprintln!("error: invalid name pattern: {err}");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    let mut selected_envs = Vec::new();

    for (_, mut venv) in riot_venvs {
        if name_regex.is_match(&venv.name).unwrap() {
            venv.shared_pkgs = shared_pkgs_map.get(&venv.name).cloned().unwrap_or_default();
            selected_envs.push(venv);
        }
    }

    Ok(selected_envs)
}
