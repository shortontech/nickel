use std::collections::{BTreeMap, BTreeSet};

/// One UTF-8 source file inside a plugin package.
#[derive(Clone, Copy, Debug)]
pub struct ModuleSource<'a> {
    pub path: &'a str,
    pub source: &'a str,
}

/// A bounded package-relative JavaScript module graph.
///
/// This deliberately supports a small ESM surface: relative static imports,
/// default and named imports, side-effect imports, and local default/named
/// exports. Modules cannot import packages, native paths, URLs, or dynamic
/// module specifiers. CSS imports participate in graph validation and are returned to the
/// existing host stylesheet compiler.
#[derive(Clone, Debug)]
pub struct JsxModuleGraph {
    entry: String,
    scripts: BTreeMap<String, String>,
    stylesheets: BTreeMap<String, String>,
    public_exports: BTreeMap<String, (String, String)>,
    component_bridge: bool,
}

impl JsxModuleGraph {
    pub fn new<'a>(
        entry: &str,
        modules: impl IntoIterator<Item = ModuleSource<'a>>,
    ) -> Result<Self, String> {
        let entry = normalize_path(entry)?;
        let mut scripts = BTreeMap::new();
        let mut stylesheets = BTreeMap::new();
        for module in modules {
            let path = normalize_path(module.path)?;
            let target = if path.ends_with(".css") {
                &mut stylesheets
            } else if path.ends_with(".js") || path.ends_with(".jsx") {
                &mut scripts
            } else {
                return Err(format!("unsupported module type {path:?}"));
            };
            if target
                .insert(path.clone(), module.source.to_owned())
                .is_some()
            {
                return Err(format!("duplicate module {path:?}"));
            }
        }
        if !scripts.contains_key(&entry) {
            return Err(format!("entry module {entry:?} is missing"));
        }
        let graph = Self {
            entry,
            scripts,
            stylesheets,
            public_exports: BTreeMap::new(),
            component_bridge: false,
        };
        graph.validate_reachable()?;
        Ok(graph)
    }

    /// Public component lookups become declarative host mount requests. The
    /// host resolves the contract outside JavaScript and invokes its owner.
    pub(crate) fn with_component_bridge(mut self) -> Self {
        self.component_bridge = true;
        self
    }

    /// Connect public contracts to actual module exports, including modules not
    /// imported by the entry. These roots share the same module cache and CSS.
    pub fn with_public_exports(
        mut self,
        exports: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        if exports.len() > 128 {
            return Err("too many public component exports".into());
        }
        for (contract, implementation) in exports {
            if contract.is_empty() || contract.len() > 128 {
                return Err("invalid public component contract".into());
            }
            let (path, name) = implementation
                .split_once('#')
                .ok_or("public component requires a module#export reference")?;
            let path = normalize_path(path)?;
            let mut chars = name.chars();
            if !chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            {
                return Err("invalid public component export name".into());
            }
            self.public_exports
                .insert(contract.clone(), (path, name.into()));
        }
        self.validate_reachable()?;
        Ok(self)
    }

    fn visit_roots(
        &self,
        visited: &mut BTreeSet<String>,
        scripts: &mut Vec<String>,
        css: &mut Vec<String>,
    ) -> Result<(), String> {
        self.visit(&self.entry, visited, scripts, css)?;
        for (path, _) in self.public_exports.values() {
            self.visit(path, visited, scripts, css)?;
        }
        Ok(())
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// CSS in deterministic dependency order for the existing CSS compiler.
    pub fn stylesheet(&self) -> Result<String, String> {
        let mut visited = BTreeSet::new();
        let mut css = Vec::new();
        self.visit_roots(&mut visited, &mut Vec::new(), &mut css)?;
        Ok(css
            .into_iter()
            .filter_map(|path| self.stylesheets.get(&path))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub(crate) fn compile(&self) -> Result<String, String> {
        let mut visited = BTreeSet::new();
        let mut order = Vec::new();
        self.visit_roots(&mut visited, &mut order, &mut Vec::new())?;
        let mut output = String::from(
            "const __nickelModules = new Map();\nconst __nickelModuleCache = new Map();\n\
             function __nickelDefineModule(id, factory) { __nickelModules.set(id, factory); }\n\
             function __nickelRequireModule(id) {\n\
               if (__nickelModuleCache.has(id)) return __nickelModuleCache.get(id).exports;\n\
               const factory = __nickelModules.get(id);\n\
               if (!factory) throw Error(`unknown module ${id}`);\n\
               const module = {exports: {}}; __nickelModuleCache.set(id, module);\n\
               factory(module, module.exports, __nickelRequireModule); return module.exports;\n}\n",
        );
        if self.component_bridge {
            output.push_str(r#"
const __nickelCompositionClient = Object.freeze({...nickel, get data() { return nickel.data; }, component(contract) {
    if (typeof contract !== 'string') throw TypeError('invalid component contract');
    return function HostComponent(props) {
        function check(value) {
            if (typeof value === 'function' || typeof value === 'symbol')
                throw TypeError('cross-package executable props require opaque callback transport');
            if (value && typeof value === 'object') {
                if (value.kind && Object.keys(value).some(key => key === 'action' || key.endsWith('Action')))
                    throw TypeError('cross-package rendered children require owned callback transport');
                for (const item of Object.values(value)) check(item);
            }
        }
        check(props);
        return {kind:'__packageComponent', contract, props};
    };
}});
"#);
        }
        for path in order {
            let source = &self.scripts[&path];
            let transformed = transform_module(&path, source)?;
            output.push_str(&format!(
                "__nickelDefineModule({}, function(module, exports, require) {{\n{}\n}});\n",
                js_string(&path),
                if self.component_bridge {
                    format!("const nickel = __nickelCompositionClient;\n{transformed}")
                } else {
                    transformed
                }
            ));
        }
        for (contract, (path, name)) in &self.public_exports {
            output.push_str(&format!(
                "__nickelPublishComponent({}, __nickelRequireModule({})[{}]);\n",
                js_string(contract),
                js_string(path),
                js_string(name)
            ));
        }
        output.push_str(&format!(
            "const __nickelEntryModule = __nickelRequireModule({});\nvar App = __nickelEntryModule.default ?? __nickelEntryModule.App;\nif (typeof App !== 'function') throw Error('entry module must export a default component or named App');\n",
            js_string(&self.entry)
        ));
        Ok(output)
    }

    fn validate_reachable(&self) -> Result<(), String> {
        self.visit_roots(&mut BTreeSet::new(), &mut Vec::new(), &mut Vec::new())
    }

    fn visit(
        &self,
        path: &str,
        visited: &mut BTreeSet<String>,
        scripts: &mut Vec<String>,
        css: &mut Vec<String>,
    ) -> Result<(), String> {
        if !visited.insert(path.to_owned()) {
            return Ok(());
        }
        let source = self
            .scripts
            .get(path)
            .ok_or_else(|| format!("script module {path:?} is missing"))?;
        for specifier in import_specifiers(source)? {
            let resolved = resolve(path, &specifier)?;
            if resolved.ends_with(".css") {
                if !self.stylesheets.contains_key(&resolved) {
                    return Err(format!("stylesheet module {resolved:?} is missing"));
                }
                if visited.insert(resolved.clone()) {
                    css.push(resolved);
                }
            } else {
                if !self.scripts.contains_key(&resolved) {
                    return Err(format!("script module {resolved:?} is missing"));
                }
                self.visit(&resolved, visited, scripts, css)?;
            }
        }
        scripts.push(path.to_owned());
        Ok(())
    }
}

fn transform_module(path: &str, source: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut exported = Vec::new();
    for raw in source.lines() {
        let line = raw.trim();
        if line.starts_with("import ") {
            output.push_str(&transform_import(path, line)?);
            output.push('\n');
        } else if let Some(rest) = line.strip_prefix("export default ") {
            output.push_str("exports.default = ");
            output.push_str(rest);
            output.push('\n');
        } else if let Some(rest) = line.strip_prefix("export ") {
            if rest.starts_with('{') {
                let list = rest
                    .trim_end_matches(';')
                    .strip_prefix('{')
                    .and_then(|s| s.strip_suffix('}'))
                    .ok_or_else(|| "named export must occupy one line".to_string())?;
                for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let mut words = item.split_whitespace();
                    let local = words.next().unwrap();
                    let exported_name = match (words.next(), words.next(), words.next()) {
                        (None, None, None) => local,
                        (Some("as"), Some(name), None) => name,
                        _ => return Err(format!("invalid named export {item:?}")),
                    };
                    output.push_str(&format!("exports.{exported_name} = {local};\n"));
                }
            } else {
                let name = declaration_name(rest)
                    .ok_or_else(|| format!("unsupported export declaration {line:?}"))?;
                output.push_str(rest);
                output.push('\n');
                exported.push(name.to_owned());
            }
        } else {
            output.push_str(raw);
            output.push('\n');
        }
    }
    for name in exported {
        output.push_str(&format!("exports.{name} = {name};\n"));
    }
    Ok(output)
}

fn transform_import(importer: &str, line: &str) -> Result<String, String> {
    let declaration = line.trim_end_matches(';').trim();
    let (binding, specifier) = if let Some(specifier) = declaration
        .strip_prefix("import '")
        .and_then(|s| s.strip_suffix('\''))
    {
        ("", specifier)
    } else if let Some(specifier) = declaration
        .strip_prefix("import \"")
        .and_then(|s| s.strip_suffix('"'))
    {
        ("", specifier)
    } else {
        let body = declaration.strip_prefix("import ").unwrap();
        let (binding, quoted) = body
            .rsplit_once(" from ")
            .ok_or_else(|| format!("invalid import {line:?}"))?;
        (binding.trim(), unquote(quoted.trim())?)
    };
    let resolved = resolve(importer, specifier)?;
    if resolved.ends_with(".css") {
        if binding.is_empty() {
            return Ok(String::new());
        }
        return Err("CSS imports cannot bind a value".into());
    }
    let require = format!("require({})", js_string(&resolved));
    if binding.is_empty() {
        return Ok(format!("{require};"));
    }
    if let Some(namespace) = binding.strip_prefix("* as ") {
        return Ok(format!("const {} = {require};", namespace.trim()));
    }
    if binding.starts_with('{') {
        let names = binding
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .ok_or("invalid named import")?;
        let names = names
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|item| item.replace(" as ", ": "))
            .collect::<Vec<_>>()
            .join(", ");
        return Ok(format!("const {{{names}}} = {require};"));
    }
    if binding.contains(',') {
        return Err("combined default and named imports are not supported".into());
    }
    Ok(format!("const {binding} = {require}.default;"))
}

pub(crate) fn import_specifiers(source: &str) -> Result<Vec<String>, String> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("import "))
        .map(|line| {
            let line = line.trim_end_matches(';');
            if let Some((_, quoted)) = line.rsplit_once(" from ") {
                unquote(quoted.trim()).map(str::to_owned)
            } else {
                unquote(line.strip_prefix("import ").unwrap().trim()).map(str::to_owned)
            }
        })
        .collect()
}

fn unquote(value: &str) -> Result<&str, String> {
    value
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| value.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .ok_or_else(|| format!("module specifier must be a quoted string: {value:?}"))
}

fn declaration_name(source: &str) -> Option<&str> {
    ["const ", "let ", "var ", "function ", "class "]
        .into_iter()
        .find_map(|prefix| {
            source.strip_prefix(prefix).and_then(|rest| {
                rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                    .next()
            })
        })
        .filter(|name| !name.is_empty())
}

pub(crate) fn resolve(importer: &str, specifier: &str) -> Result<String, String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return Err(format!(
            "module import must be package-relative: {specifier:?}"
        ));
    }
    let parent = importer.rsplit_once('/').map_or("", |(parent, _)| parent);
    let joined = if parent.is_empty() {
        specifier.to_owned()
    } else {
        format!("{parent}/{specifier}")
    };
    normalize_path(&joined)
}

pub(crate) fn normalize_path(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\\') || path.contains(':') || path.starts_with('/') {
        return Err(format!("invalid module path {path:?}"));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("module path escapes package: {path:?}"));
                }
            }
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err(format!("invalid module path {path:?}"));
    }
    Ok(parts.join("/"))
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).expect("module path is serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JsxRuntime;

    #[test]
    fn imported_components_share_one_runtime_and_module_instance() {
        let graph = JsxModuleGraph::new("main.js", [
            ModuleSource { path: "main.js", source: "import Counter from './counter.js';\nimport './theme.css';\nexport default function App() { return h(Window, {}, h(Counter)); }" },
            ModuleSource { path: "counter.js", source: "let loads = 0;\nloads += 1;\nexport default function Counter() { const [count, setCount] = useState(0); return h(Button, {onClick: () => setCount(count + loads)}, String(count)); }" },
            ModuleSource { path: "theme.css", source: ".root { color: #fff; }" },
        ]).unwrap();
        assert_eq!(graph.stylesheet().unwrap(), ".root { color: #fff; }");
        let mut runtime = JsxRuntime::new_modules(&graph, None).unwrap();
        let first: serde_json::Value = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(first.to_string().contains("0"));
        let second: serde_json::Value = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert!(second.to_string().contains("1"));
    }

    #[test]
    fn rejects_external_and_escaping_imports() {
        for specifier in ["react", "../../outside.js", "/tmp/code.js"] {
            let source = format!(
                "import value from '{specifier}';\nexport default function App() {{ return value; }}"
            );
            assert!(
                JsxModuleGraph::new(
                    "ui/main.js",
                    [ModuleSource {
                        path: "ui/main.js",
                        source: &source
                    }]
                )
                .is_err()
            );
        }
    }

    #[test]
    fn public_component_roots_share_module_identity_and_load_their_css() {
        let graph = JsxModuleGraph::new("main.js", [
            ModuleSource { path: "main.js", source: "export function App() { return h(nickel.component('shell.widget'), {}); }" },
            ModuleSource { path: "widget.js", source: "import './widget.css';\nlet calls = 0;\nexport function Widget() { return h(Text, {}, 'widget ' + ++calls); }" },
            ModuleSource { path: "widget.css", source: ".widget { padding: 8px; }" },
        ]).unwrap().with_public_exports(&BTreeMap::from([
            ("shell.widget".into(), "./widget.js#Widget".into()),
            ("shell.widget.alias".into(), "./widget.js#Widget".into()),
        ])).unwrap();
        assert!(graph.stylesheet().unwrap().contains("padding: 8px"));
        let mut runtime = JsxRuntime::new_modules(&graph, None).unwrap();
        assert!(runtime.eval_json::<bool>("JSON.stringify(nickel.component('shell.widget') === nickel.component('shell.widget.alias'))").unwrap());
        let rendered: serde_json::Value = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(rendered.to_string().contains("widget 1"));
        assert!(runtime.eval("nickel.component('missing')").is_err());
    }

    #[test]
    fn public_component_references_reject_missing_modules_and_non_components() {
        let graph = JsxModuleGraph::new("main.js", [ModuleSource {
            path: "main.js", source: "export const label = 'text';\nexport function App() { return h(Text, {}, 'app'); }",
        }]).unwrap();
        assert!(
            graph
                .clone()
                .with_public_exports(&BTreeMap::from([(
                    "shell.bad".into(),
                    "./missing.js#Widget".into()
                )]))
                .is_err()
        );
        let graph = graph
            .with_public_exports(&BTreeMap::from([(
                "shell.bad".into(),
                "./main.js#label".into(),
            )]))
            .unwrap();
        assert!(JsxRuntime::new_modules(&graph, None).is_err());
    }

    #[test]
    fn named_and_namespace_imports_work() {
        let graph = JsxModuleGraph::new("main.js", [
            ModuleSource { path: "main.js", source: "import {label as text} from './values.js';\nimport * as values from './values.js';\nexport function App() { return h(Text, {}, text + values.suffix); }" },
            ModuleSource { path: "values.js", source: "export const label = 'Nickel';\nexport const suffix = '!';" },
        ]).unwrap();
        let mut runtime = JsxRuntime::new_modules(&graph, None).unwrap();
        let rendered: serde_json::Value = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(rendered.to_string().contains("Nickel!"));
    }
}
