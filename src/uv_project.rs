use std::{
    fmt::Write as _,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use toml::{Table, Value};

use crate::{
    constants::{RT_VERSION, RUNTIME_DISTRIBUTION, UV_PROJECTS_DIR},
    error::{RtError, RtResult},
    venv::{TestEnvironment, requirement_name},
};

/// Generate the disposable uv project used to materialize one test environment.
pub fn ensure_uv_project(
    cache_root: &Path,
    project_root: &Path,
    runtime_project: &Path,
    environment: &TestEnvironment,
    _no_editable: bool,
) -> RtResult<PathBuf> {
    let project_root = fs::canonicalize(project_root).map_err(|err| {
        RtError::message(format!(
            "error: failed to resolve project root {}: {err}",
            project_root.display()
        ))
    })?;
    let runtime_project = fs::canonicalize(runtime_project).map_err(|err| {
        RtError::message(format!(
            "error: failed to resolve rt runtime project {}: {err}",
            runtime_project.display()
        ))
    })?;
    let local_project_name = Some(read_project_name(&project_root)?);
    let mut requirements =
        environment_requirements(environment, local_project_name.as_deref(), &project_root);
    requirements.push(format!("{RUNTIME_DISTRIBUTION}=={RT_VERSION}"));
    let local_project = local_project_name
        .as_deref()
        .map(|name| (name, project_root.as_path()));
    let project_id = project_id(
        &environment.python,
        &requirements,
        local_project,
        &runtime_project,
    );
    let generated_name = format!("rt-generated-{}", &project_id[..16]);
    let contents = render_project(
        &generated_name,
        &environment.python,
        &requirements,
        local_project,
        &runtime_project,
    )?;

    let project_dir = cache_root.join(UV_PROJECTS_DIR).join(project_id);
    fs::create_dir_all(&project_dir)?;

    let pyproject_path = project_dir.join("pyproject.toml");
    write_if_changed(&pyproject_path, &contents)?;

    Ok(project_dir)
}

fn project_id(
    python: &str,
    requirements: &[String],
    local_project: Option<(&str, &Path)>,
    runtime_project: &Path,
) -> String {
    fn update(hasher: &mut Sha256, value: &[u8]) {
        let length = u64::try_from(value.len()).expect("project metadata length fits in u64");
        hasher.update(length.to_le_bytes());
        hasher.update(value);
    }

    let mut hasher = Sha256::new();
    update(&mut hasher, b"rt-uv-project-v1");
    update(&mut hasher, python.as_bytes());
    for requirement in requirements {
        update(&mut hasher, requirement.as_bytes());
    }
    if let Some((name, path)) = local_project {
        update(&mut hasher, b"editable");
        update(&mut hasher, name.as_bytes());
        update(&mut hasher, path.to_string_lossy().as_bytes());
    } else {
        update(&mut hasher, b"no-local-project");
    }
    update(&mut hasher, b"rt-runtime");
    update(&mut hasher, runtime_project.to_string_lossy().as_bytes());

    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
}

fn write_if_changed(path: &Path, contents: &str) -> io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(());
    }

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("generated project path has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents.as_bytes())?;
    temporary.flush()?;
    temporary.persist(path).map_err(|err| err.error)?;
    Ok(())
}

fn read_project_name(project_root: &Path) -> RtResult<String> {
    let path = project_root.join("pyproject.toml");
    let contents = fs::read_to_string(&path).map_err(|err| {
        RtError::message(format!("error: failed to read {}: {err}", path.display()))
    })?;
    let parsed: Value = toml::from_str(&contents).map_err(|err| {
        RtError::message(format!("error: failed to parse {}: {err}", path.display()))
    })?;

    parsed
        .get("project")
        .and_then(|project| project.get("name"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            RtError::message(format!(
                "error: {} must define project.name for the editable install",
                path.display()
            ))
        })
}

fn environment_requirements(
    venv: &TestEnvironment,
    project_name: Option<&str>,
    project_root: &Path,
) -> Vec<String> {
    let mut requirements = Vec::new();

    if let Some(project_name) = project_name {
        requirements.push(project_name.to_owned());
    }

    let packages: Vec<String> = if venv.resolved_pkgs.is_empty() {
        venv.direct_dependencies.clone()
    } else {
        venv.resolved_pkgs
            .iter()
            .map(|(name, version)| format!("{name}=={version}"))
            .collect()
    };

    let project_name = project_name.map(normalize_pkg_name);
    let project_root = project_root.to_string_lossy();
    requirements.extend(packages.into_iter().filter_map(|requirement| {
        let name = requirement_name(&requirement);
        if project_name.as_deref() == Some(name.as_str()) {
            None
        } else {
            Some(requirement.replace("/home/bits/project", &project_root))
        }
    }));

    if has_pkg(venv, "mock") && !has_pkg(venv, "types-mock") {
        requirements.push("types-mock".to_owned());
    }

    requirements
}

fn render_project(
    generated_name: &str,
    python: &str,
    requirements: &[String],
    local_project: Option<(&str, &Path)>,
    runtime_project: &Path,
) -> RtResult<String> {
    let mut project = Table::new();
    project.insert("name".to_owned(), Value::String(generated_name.to_owned()));
    project.insert("version".to_owned(), Value::String("0.0.0".to_owned()));
    project.insert(
        "requires-python".to_owned(),
        Value::String(requires_python(python)?),
    );
    project.insert(
        "dependencies".to_owned(),
        Value::Array(requirements.iter().cloned().map(Value::String).collect()),
    );

    let mut uv = Table::new();
    uv.insert("package".to_owned(), Value::Boolean(false));
    let mut sources = Table::new();
    if let Some((name, path)) = local_project {
        let mut source = Table::new();
        source.insert(
            "path".to_owned(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        source.insert("editable".to_owned(), Value::Boolean(true));

        sources.insert(name.to_owned(), Value::Table(source));
    }

    let mut runtime_source = Table::new();
    runtime_source.insert(
        "path".to_owned(),
        Value::String(runtime_project.to_string_lossy().into_owned()),
    );
    runtime_source.insert("editable".to_owned(), Value::Boolean(false));
    sources.insert(
        RUNTIME_DISTRIBUTION.to_owned(),
        Value::Table(runtime_source),
    );
    uv.insert("sources".to_owned(), Value::Table(sources));

    let mut tool = Table::new();
    tool.insert("uv".to_owned(), Value::Table(uv));

    let mut root = Table::new();
    root.insert("project".to_owned(), Value::Table(project));
    root.insert("tool".to_owned(), Value::Table(tool));

    let body = toml::to_string_pretty(&Value::Table(root))
        .map_err(|err| RtError::message(format!("error: failed to generate uv project: {err}")))?;
    Ok(format!(
        "# Generated by rt from suitespec; do not edit.\n{body}"
    ))
}

fn requires_python(python: &str) -> RtResult<String> {
    let mut components = python.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let minor = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    match (major, minor) {
        (Some(major), Some(minor)) => Ok(format!(">={major}.{minor},<{major}.{}", minor + 1)),
        _ => Err(RtError::message(format!(
            "error: expected a major.minor Python version, got {python:?}"
        ))),
    }
}

/// Normalize a requirement name the way PEP 503 does, so `Types_Mock` and
/// `types-mock` compare equal.
fn normalize_pkg_name(name: &str) -> String {
    name.split('[')
        .next()
        .unwrap_or(name)
        .trim()
        .to_lowercase()
        .replace(['_', '.'], "-")
}

/// Whether `pkg` is direct or present in the committed suitespec lockfile.
pub fn has_pkg(venv: &TestEnvironment, pkg: &str) -> bool {
    let target = normalize_pkg_name(pkg);
    venv.direct_dependencies
        .iter()
        .any(|requirement| requirement_name(requirement) == target)
        || venv
            .resolved_pkgs
            .keys()
            .any(|name| normalize_pkg_name(name) == target)
}

#[cfg(test)]
mod tests {
    use super::{UV_PROJECTS_DIR, ensure_uv_project, normalize_pkg_name};
    use crate::constants::{RT_VERSION, RUNTIME_DISTRIBUTION};
    use crate::venv::TestEnvironment;
    use std::{fs, path::PathBuf};

    fn fixture() -> (tempfile::TempDir, PathBuf, TestEnvironment) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"example-project\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let runtime_project = dir.path().join("rt-runtime");
        fs::create_dir(&runtime_project).unwrap();

        let mut venv = TestEnvironment::empty_for_tests();
        venv.python = "3.11".to_owned();
        venv.hash = "abc1234".to_owned();
        venv.direct_dependencies.push("pytest>=8,<9".to_owned());
        (dir, runtime_project, venv)
    }

    #[test]
    fn generated_project_contains_dependencies_and_an_editable_local_source() {
        let (dir, runtime_project, venv) = fixture();
        let cache_root = dir.path().join(".cache/rt");

        let generated =
            ensure_uv_project(&cache_root, dir.path(), &runtime_project, &venv, false).unwrap();
        assert_eq!(
            generated.parent(),
            Some(cache_root.join(UV_PROJECTS_DIR).as_path())
        );
        assert_eq!(generated.file_name().unwrap().to_string_lossy().len(), 64);
        let contents = fs::read_to_string(generated.join("pyproject.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&contents).unwrap();

        let dependencies = parsed["project"]["dependencies"].as_array().unwrap();
        assert_eq!(dependencies[0].as_str(), Some("example-project"));
        assert_eq!(dependencies[1].as_str(), Some("pytest>=8,<9"));
        assert_eq!(
            dependencies[2].as_str(),
            Some(format!("{RUNTIME_DISTRIBUTION}=={RT_VERSION}").as_str())
        );
        assert_eq!(
            parsed["tool"]["uv"]["sources"]["example-project"]["path"].as_str(),
            Some(
                dir.path()
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            parsed["tool"]["uv"]["sources"]["example-project"]["editable"].as_bool(),
            Some(true)
        );
        assert_eq!(
            parsed["tool"]["uv"]["sources"][RUNTIME_DISTRIBUTION]["path"].as_str(),
            Some(
                runtime_project
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            parsed["tool"]["uv"]["sources"][RUNTIME_DISTRIBUTION]["editable"].as_bool(),
            Some(false)
        );
        assert_eq!(
            parsed["project"]["requires-python"].as_str(),
            Some(">=3.11,<3.12")
        );
    }

    #[test]
    fn dependency_changes_use_a_different_project() {
        let (dir, runtime_project, mut venv) = fixture();
        let cache_root = dir.path().join(".cache/rt");

        let first =
            ensure_uv_project(&cache_root, dir.path(), &runtime_project, &venv, false).unwrap();
        venv.direct_dependencies.push("coverage==7.10.0".to_owned());
        let second =
            ensure_uv_project(&cache_root, dir.path(), &runtime_project, &venv, false).unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn runtime_source_changes_use_a_different_project() {
        let (dir, runtime_project, venv) = fixture();
        let cache_root = dir.path().join(".cache/rt");
        let other_runtime_project = dir.path().join("other-rt-runtime");
        fs::create_dir(&other_runtime_project).unwrap();

        let first =
            ensure_uv_project(&cache_root, dir.path(), &runtime_project, &venv, false).unwrap();
        let second = ensure_uv_project(
            &cache_root,
            dir.path(),
            &other_runtime_project,
            &venv,
            false,
        )
        .unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn a_damaged_generated_manifest_is_replaced() {
        let (dir, runtime_project, venv) = fixture();
        let cache_root = dir.path().join(".cache/rt");
        let generated =
            ensure_uv_project(&cache_root, dir.path(), &runtime_project, &venv, false).unwrap();
        let manifest = generated.join("pyproject.toml");
        fs::write(&manifest, "incomplete").unwrap();

        let regenerated =
            ensure_uv_project(&cache_root, dir.path(), &runtime_project, &venv, false).unwrap();

        assert_eq!(generated, regenerated);
        assert!(fs::read_to_string(manifest).unwrap().contains("[project]"));
    }

    #[test]
    fn no_editable_materialization_still_includes_the_local_project() {
        let (dir, runtime_project, venv) = fixture();
        let generated = ensure_uv_project(
            &dir.path().join(".cache/rt"),
            dir.path(),
            &runtime_project,
            &venv,
            true,
        )
        .unwrap();
        let contents = fs::read_to_string(generated.join("pyproject.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&contents).unwrap();

        let dependencies = parsed["project"]["dependencies"].as_array().unwrap();
        assert_eq!(dependencies.len(), 3);
        assert_eq!(dependencies[0].as_str(), Some("example-project"));
        assert_eq!(dependencies[1].as_str(), Some("pytest>=8,<9"));
        assert_eq!(
            dependencies[2].as_str(),
            Some(format!("{RUNTIME_DISTRIBUTION}=={RT_VERSION}").as_str())
        );
        assert_eq!(
            parsed["tool"]["uv"]["sources"][RUNTIME_DISTRIBUTION]["editable"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn resolved_requirements_win_without_duplicating_the_local_project() {
        let (dir, runtime_project, mut venv) = fixture();
        venv.direct_dependencies.push("mock".to_owned());
        venv.resolved_pkgs = [
            ("example-project".to_owned(), "0.9.0".to_owned()),
            ("mock".to_owned(), "5.2.0".to_owned()),
        ]
        .into_iter()
        .collect();

        let generated = ensure_uv_project(
            &dir.path().join(".cache/rt"),
            dir.path(),
            &runtime_project,
            &venv,
            false,
        )
        .unwrap();
        let contents = fs::read_to_string(generated.join("pyproject.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&contents).unwrap();
        let requirements = parsed["project"]["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            requirements,
            [
                "example-project",
                "mock==5.2.0",
                "types-mock",
                format!("{RUNTIME_DISTRIBUTION}=={RT_VERSION}").as_str(),
            ]
        );
    }

    #[test]
    fn package_names_are_normalized_like_pep503() {
        assert_eq!(normalize_pkg_name("Types_Mock"), "types-mock");
        assert_eq!(normalize_pkg_name("coverage[toml]"), "coverage");
        assert_eq!(normalize_pkg_name("ruamel.yaml"), "ruamel-yaml");
    }
}
