//! Installed shell composition linking. Linking is separate from activation:
//! shared JavaScript globals do not yet isolate the authority of different packages.

use std::collections::{BTreeMap, BTreeSet};

use nickel_core::package_composition::{
    PackageIdentity, ResolvedShellPackage, resolve_shell_package,
};
use nickel_core::plugins::PluginPackage;

use crate::{JsxModuleGraph, JsxRuntime, ModuleSource};

/// A validated link plan with code ownership retained outside JavaScript.
/// Cross-owner execution remains gated until the host has a trusted dispatcher
/// or isolated realms; editable JavaScript effect tags cannot establish grants.
#[derive(Clone, Debug)]
pub struct ComposedShellGraph {
    graph: JsxModuleGraph,
    pub resolution: ResolvedShellPackage,
    module_owners: BTreeMap<String, PackageIdentity>,
    associated_stylesheets: String,
}

impl ComposedShellGraph {
    pub fn link(catalog: &BTreeMap<String, PluginPackage>, active: &str) -> Result<Self, String> {
        let manifests = catalog
            .iter()
            .filter(|(_, package)| package.manifest.composition.is_some())
            .map(|(id, package)| {
                let composition = package
                    .manifest
                    .composition
                    .clone()
                    .ok_or_else(|| format!("package {id:?} has no composition manifest"))?;
                if package.manifest.id != composition.id
                    || package.manifest.version.as_deref() != Some(&composition.version)
                {
                    return Err(format!(
                        "composition identity differs from installed package {id:?}"
                    ));
                }
                Ok((id.clone(), composition))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let resolution = resolve_shell_package(&manifests, active)
            .map_err(|error| format!("shell composition failed: {error:?}"))?;
        let mut sources = BTreeMap::new();
        let mut module_owners = BTreeMap::new();
        let mut source_bytes = 0usize;
        let mut associated_stylesheets = String::new();
        for owner in &resolution.inheritance_chain {
            let package = &catalog[&owner.id];
            source_bytes = source_bytes
                .checked_add(package.stylesheet.len())
                .ok_or("composition source size overflow")?;
            if source_bytes > 16 * 1024 * 1024 {
                return Err("composition exceeds source module limits".into());
            }
            associated_stylesheets.push_str(&package.stylesheet);
            associated_stylesheets.push('\n');
            for (path, source) in package
                .modules
                .iter()
                .filter(|module| module.path != package.manifest.entry)
                .map(|module| (module.path.as_str(), module.source.as_str()))
                .chain(std::iter::once((
                    package.manifest.entry.as_str(),
                    package.source.as_str(),
                )))
            {
                source_bytes = source_bytes
                    .checked_add(source.len())
                    .ok_or("composition source size overflow")?;
                if source_bytes > 16 * 1024 * 1024 || sources.len() >= 1024 {
                    return Err("composition exceeds source module limits".into());
                }
                if twinkle_jsx_runtime::modules::normalize_path(path)?.starts_with("__public/") {
                    return Err("package source uses reserved public link namespace".into());
                }
                // Validate imports against the original package root before
                // prefixing; ../../ must never escape into another namespace.
                let mut source = source.to_owned();
                if !path.ends_with(".css") {
                    let mut replacements = Vec::new();
                    for specifier in twinkle_jsx_runtime::modules::import_specifiers(&source)? {
                        if specifier.starts_with('.') {
                            twinkle_jsx_runtime::modules::resolve(path, &specifier)?;
                        } else {
                            let (dependency, contract) = specifier
                                .split_once('/')
                                .ok_or("cross-package imports require package/public.contract")?;
                            let dependency_index = resolution
                                .inheritance_chain
                                .iter()
                                .position(|identity| identity.id == dependency);
                            let owner_index = resolution
                                .inheritance_chain
                                .iter()
                                .position(|identity| identity == owner)
                                .unwrap();
                            if dependency_index.is_none_or(|index| index >= owner_index) {
                                return Err("imports may only use public ancestor contracts".into());
                            }
                            let dependency_resolution =
                                resolve_shell_package(&manifests, dependency).map_err(|error| {
                                    format!("dependency composition failed: {error:?}")
                                })?;
                            let export = dependency_resolution
                                .exports
                                .get(contract)
                                .ok_or("cross-package import names a private or missing export")?;
                            let (module, symbol) = export
                                .implementation
                                .split_once('#')
                                .ok_or("public export requires module#symbol")?;
                            let alias = format!("{}/__public/{dependency}/{contract}.js", owner.id);
                            let target = qualified(&export.implemented_by.id, module)?;
                            let alias_source = if symbol == "default" {
                                format!(
                                    "import Component from '{}';\nexport default Component;\n",
                                    relative(&alias, &target)
                                )
                            } else {
                                format!(
                                    "import {{ {symbol} }} from '{}';\nexport {{ {symbol} }};\n",
                                    relative(&alias, &target)
                                )
                            };
                            sources.insert(alias.clone(), alias_source);
                            module_owners.insert(alias.clone(), owner.clone());
                            replacements
                                .push((specifier, relative(&qualified(&owner.id, path)?, &alias)));
                        }
                    }
                    // Replace only parsed static import declaration lines.
                    source = source
                        .lines()
                        .map(|line| {
                            if line.trim_start().starts_with("import ") {
                                replacements
                                    .iter()
                                    .fold(line.to_owned(), |line, (old, new)| {
                                        line.replace(&format!("'{old}'"), &format!("'{new}'"))
                                            .replace(&format!("\"{old}\""), &format!("\"{new}\""))
                                    })
                            } else {
                                line.to_owned()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                let path = qualified(&owner.id, path)?;
                if sources.insert(path.clone(), source).is_some() {
                    return Err(format!("duplicate or reserved composition module {path:?}"));
                }
                module_owners.insert(path, owner.clone());
            }
        }
        let entry = qualified(active, &catalog[active].manifest.entry)?;
        let exports = resolution
            .exports
            .iter()
            .map(|(contract, export)| {
                Ok((
                    contract.clone(),
                    qualified_reference(&export.implemented_by.id, &export.implementation)?,
                ))
            })
            .chain(resolution.contributions.values().flatten().map(|entry| {
                Ok((
                    format!(
                        "contribution.{}.{}.{}",
                        entry.collection, entry.contributed_by.id, entry.id
                    ),
                    qualified_reference(&entry.contributed_by.id, &entry.implementation)?,
                ))
            }))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let graph = JsxModuleGraph::new(
            &entry,
            sources
                .iter()
                .map(|(path, source)| ModuleSource { path, source }),
        )?
        .with_public_exports(&exports)?;
        Ok(Self {
            graph,
            resolution,
            module_owners,
            associated_stylesheets,
        })
    }

    pub fn module_owners(&self) -> &BTreeMap<String, PackageIdentity> {
        &self.module_owners
    }

    pub fn stylesheet(&self) -> Result<String, String> {
        Ok(format!(
            "{}\n{}",
            self.associated_stylesheets,
            self.graph.stylesheet()?
        ))
    }

    /// Temporary activation gate: composition is inspectable before authority
    /// isolation exists, but foreign code cannot inherit the active shell's grants.
    pub fn activate(&self, data: Option<&str>) -> Result<JsxRuntime, String> {
        if self.module_owners.values().collect::<BTreeSet<_>>().len() > 1 {
            return Err(
                "cross-package shell activation requires trusted effect authority isolation".into(),
            );
        }
        crate::create_module_runtime(&self.graph, data)
    }
}

fn qualified(owner: &str, path: &str) -> Result<String, String> {
    Ok(format!(
        "{owner}/{}",
        twinkle_jsx_runtime::modules::normalize_path(path)?
    ))
}
fn qualified_reference(owner: &str, reference: &str) -> Result<String, String> {
    let (path, symbol) = reference
        .split_once('#')
        .ok_or("public export requires module#symbol")?;
    Ok(format!("{}#{symbol}", qualified(owner, path)?))
}
fn relative(importer: &str, target: &str) -> String {
    let parent: Vec<_> = importer.split('/').collect();
    let target: Vec<_> = target.split('/').collect();
    let shared = parent[..parent.len() - 1]
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count();
    format!(
        "./{}{}",
        "../".repeat(parent.len() - 1 - shared),
        target[shared..].join("/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::plugins::PluginManifest;

    fn package(id: &str, source: &str, base: Option<&str>) -> PluginPackage {
        let mut manifest = PluginManifest::from_json(include_str!(
            "../../../assets/plugins/nickel-default/plugin.json"
        ))
        .unwrap();
        manifest.id = id.into();
        manifest.entry = "main.js".into();
        let composition = manifest.composition.as_mut().unwrap();
        composition.id = id.into();
        composition.exports =
            BTreeMap::from([("shell.taskbar".into(), "./main.js#Taskbar".into())]);
        if let Some(base) = base {
            composition.exports.clear();
            composition.extends = Some(base.into());
            composition.requires.insert(base.into(), "^0.2".into());
        }
        PluginPackage {
            manifest,
            source: source.into(),
            stylesheet: String::new(),
            modules: Vec::new(),
            images: BTreeMap::new(),
        }
    }

    #[test]
    fn links_public_dependency_and_preserves_code_owner_before_activation() {
        let base = package(
            "base",
            "export function Taskbar() { return h(Text, null, 'base'); }\nexport default function App() { return h(Taskbar, null); }",
            None,
        );
        let child = package(
            "child",
            "import { Taskbar } from 'base/shell.taskbar';\nexport default function App() { return h(Taskbar, null); }",
            Some("base"),
        );
        let plan = ComposedShellGraph::link(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
        )
        .unwrap();
        assert_eq!(
            plan.resolution.exports["shell.taskbar"].implemented_by.id,
            "base"
        );
        assert_eq!(plan.module_owners()["base/main.js"].id, "base");
        assert_eq!(plan.module_owners()["child/main.js"].id, "child");
        assert!(plan.graph.compile().unwrap().contains("base/main.js"));
        assert!(
            plan.activate(None)
                .err()
                .unwrap()
                .contains("authority isolation")
        );
    }

    #[test]
    fn rejects_private_imports_and_namespace_escape() {
        for source in [
            "import { Taskbar } from 'base/main.js';\nexport default function App() {}",
            "import { Taskbar } from '../base/main.js';\nexport default function App() {}",
        ] {
            let base = package(
                "base",
                "export function Taskbar() {}\nexport default function App() {}",
                None,
            );
            let child = package("child", source, Some("base"));
            assert!(
                ComposedShellGraph::link(
                    &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
                    "child"
                )
                .is_err()
            );
        }
    }

    #[test]
    fn replacement_and_contribution_roots_share_the_linked_graph() {
        let base = package(
            "base",
            "export function Taskbar() {}\nexport default function App() {}",
            None,
        );
        let mut child = package(
            "child",
            "export function Taskbar() {}\nexport function Page() {}\nexport default function App() {}",
            Some("base"),
        );
        let composition = child.manifest.composition.as_mut().unwrap();
        composition
            .replaces
            .insert("shell.taskbar".into(), "./main.js#Taskbar".into());
        composition
            .contributions
            .push(nickel_core::package_composition::SemanticContribution {
                collection: "settings.pages".into(),
                id: "theme".into(),
                implementation: "./main.js#Page".into(),
                priority: 0,
            });
        let plan = ComposedShellGraph::link(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
        )
        .unwrap();
        assert_eq!(
            plan.resolution.exports["shell.taskbar"].implemented_by.id,
            "child"
        );
        assert_eq!(
            plan.resolution.contributions["settings.pages"][0]
                .contributed_by
                .id,
            "child"
        );
        let compiled = plan.graph.compile().unwrap();
        assert!(compiled.contains("contribution.settings.pages.child.theme"));
        assert_eq!(
            compiled
                .matches("__nickelDefineModule(\"child/main.js\"")
                .count(),
            1
        );
    }

    #[test]
    fn ordinary_single_package_activates_in_one_context() {
        let base = package(
            "base",
            "export function Taskbar() { return h(Text, null, 'base'); }\nexport default function App() { return h(Taskbar, null); }",
            None,
        );
        let plan =
            ComposedShellGraph::link(&BTreeMap::from([("base".into(), base)]), "base").unwrap();
        let mut runtime = plan.activate(None).unwrap();
        let value: serde_json::Value = runtime
            .eval_json("JSON.stringify(__nickelRender())")
            .unwrap();
        assert!(value.to_string().contains("base"));
    }
}
