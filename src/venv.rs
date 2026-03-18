use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fancy_regex::Regex;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use sha2::{Digest, Sha256};
use shell_words::split;

use crate::{
    config::Selector,
    config_provider::{ConfigProvider, ProviderServices, ProviderVenvNode},
    constants::VENV_PREFIX,
    error::{RtError, RtResult},
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    pub command: Option<String>,
    pub pytest_target: Option<String>,
    pub env: IndexMap<String, String>,
    pub create: bool,
    pub skip_dev_install: bool,
    pub hash: String,
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

#[must_use]
pub fn venv_path(riot_root: &Path, short_hash: &str) -> PathBuf {
    riot_root.join(format!("{}{}", VENV_PREFIX, short_hash.replace('@', "_")))
}

#[must_use]
pub fn venv_python_path(riot_root: &Path, short_hash: &str) -> String {
    venv_path(riot_root, short_hash)
        .join("bin/python")
        .to_string_lossy()
        .to_string()
}

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
    fn merge(&self, venv: &ProviderVenvNode) -> Option<Self> {
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

        if let Some(values) = pys.as_ref()
            && values.is_empty()
        {
            return None;
        }
        next.pys = pys;

        Some(next)
    }
}

pub fn load_context<P: ConfigProvider>(
    riotfile_path: &Path,
) -> RtResult<IndexMap<String, RiotVenv>> {
    let loaded = P::load(riotfile_path)?;
    Ok(normalize_venvs(&loaded.root, loaded.services.as_ref()))
}

fn normalize_venvs(
    root: &ProviderVenvNode,
    service_map: Option<&ProviderServices>,
) -> IndexMap<String, RiotVenv> {
    let mut venvs = IndexMap::new();
    collect_riot_venvs(root, &ResolvedSpec::default(), &mut venvs, service_map);
    for venv in venvs.values_mut() {
        venv.shared_env = shared_entries(venv.execution_contexts.iter().map(|ctx| &ctx.env));
    }
    venvs
}

fn collect_riot_venvs(
    venv: &ProviderVenvNode,
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
                let interpreter_repr = interpreter_repr(py_version);
                for pkgs in &pkg_variants {
                    let full_pkg_str = pip_deps(pkgs);
                    let name_repr = python_repr_str(name);
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
        collect_riot_venvs(child, &next_state, acc, service_map);
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
        shared.retain(|key, value| map.get(key).is_some_and(|other| other == value));
    }
    shared
}

fn pip_deps(pkgs: &IndexMap<String, String>) -> String {
    let mut parts = Vec::with_capacity(pkgs.len());
    for (lib, version) in pkgs {
        parts.push(format!("'{lib}{version}'"));
    }
    parts.join(" ")
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

#[must_use]
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

fn python_repr_str(value: &str) -> String {
    fn build(input: &str, quote: char) -> String {
        let mut out = String::new();
        out.push(quote);
        for ch in input.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\x08' => out.push_str("\\b"),
                '\x0c' => out.push_str("\\f"),
                c if c == quote => {
                    out.push('\\');
                    out.push(c);
                }
                c if (c as u32) < 0x20 || c == '\x7f' => {
                    use std::fmt::Write;
                    write!(&mut out, "\\x{:02x}", c as u32).ok();
                }
                c => out.push(c),
            }
        }
        out.push(quote);
        out
    }

    let single = build(value, '\'');
    let double = build(value, '"');
    let single_escapes = single.matches("\\'").count();
    let double_escapes = double.matches("\\\"").count();

    if double_escapes < single_escapes {
        double
    } else {
        single
    }
}

fn interpreter_repr(py_hint: &str) -> String {
    format!("Interpreter(_hint={})", python_repr_str(py_hint))
}

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

        let mut hash_value = remainder.cast_signed();
        if hash_value == -1 {
            hash_value = -2;
        }

        let hex_str = format!("{hash_value:#x}");
        extract_hash(&hex_str)
    }

    fn context_hash(
        command: Option<&String>,
        env: &IndexMap<String, String>,
        create: bool,
        skip_dev_install: bool,
    ) -> String {
        let command_repr =
            command.map_or_else(|| "None".to_string(), |value| python_repr_str(value));

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

        let Some(path_token) = token.split("::").next() else {
            continue;
        };
        let candidate = PathBuf::from(path_token);

        if (candidate.is_dir() || candidate.extension().is_some_and(|ext| ext == "py"))
            && candidate.exists()
        {
            return Some(token.to_string());
        }
    }

    None
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
    mut venvs: IndexMap<String, RiotVenv>,
    selector: Selector,
) -> RtResult<Vec<RiotVenv>> {
    let (pattern_selector, python_selector) = match selector {
        Selector::Pattern(pattern) => (pattern, None),
        Selector::Generic { python, pattern } => (pattern.unwrap_or_default(), python),
    };

    if let Some(python_selector) = python_selector {
        let selected: IndexSet<_> = python_selector.into_iter().collect();
        venvs.retain(|_, venv| selected.contains(&venv.python));
    }
    let shared_pkgs_map = shared_pkgs_by_name(venvs.values());

    if is_short_hash(&pattern_selector) {
        let Some(mut venv) = venvs.get(&pattern_selector).cloned() else {
            return Ok(vec![]);
        };
        venv.shared_pkgs = shared_pkgs_map.get(&venv.name).cloned().unwrap_or_default();
        return Ok(vec![venv]);
    }

    if let Some(venv_hash) = parse_ctx_hash(&pattern_selector) {
        let Some(mut venv) = venvs.get(venv_hash).cloned() else {
            return Ok(vec![]);
        };

        let shared_env = venv.shared_env.clone();
        venv.shared_pkgs = shared_pkgs_map.get(&venv.name).cloned().unwrap_or_default();
        venv.execution_contexts
            .retain(|ctx| ctx.hash == pattern_selector);
        venv.shared_env = shared_env;
        return Ok(vec![venv]);
    }

    let name_regex = Regex::new(&pattern_selector)
        .map_err(|err| RtError::message(format!("error: invalid name pattern: {err}")))?;

    let mut selected_envs = Vec::new();

    for (_, mut venv) in venvs {
        if name_regex.is_match(&venv.name).map_err(|err| {
            RtError::message(format!("error: failed to evaluate name pattern: {err}"))
        })? {
            venv.shared_pkgs = shared_pkgs_map.get(&venv.name).cloned().unwrap_or_default();
            selected_envs.push(venv);
        }
    }

    Ok(selected_envs)
}

#[cfg(test)]
mod tests {
    use super::parse_pytest_target;

    #[test]
    fn parse_pytest_target_keeps_pytest_node_id() {
        let target =
            parse_pytest_target("pytest tests/data/simple_riotfile.py::Test_Django {cmdargs}");

        assert_eq!(
            target.as_deref(),
            Some("tests/data/simple_riotfile.py::Test_Django"),
        );
    }
}
