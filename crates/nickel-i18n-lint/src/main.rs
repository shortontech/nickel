use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use proc_macro2::Span;
use syn::{
    Attribute, Expr, ExprCall, ExprMacro, ExprMethodCall, FnArg, ImplItemFn, ItemFn, ItemMod,
    ItemStruct, Lit, Macro, Member, Pat, Token,
    punctuated::Punctuated,
    visit::{self, Visit},
};

const CODE: &str = "NIL001";
const SUPPRESSION: &str = "nickel-i18n-lint: allow";

#[derive(Debug, Eq, PartialEq)]
struct Diagnostic {
    line: usize,
    column: usize,
    literal: String,
    sink: String,
}

struct UiStringVisitor<'a> {
    lines: Vec<&'a str>,
    diagnostics: Vec<Diagnostic>,
    bindings: Vec<HashMap<String, ValueFact>>,
    wrapper_parameters: &'a HashMap<String, HashSet<usize>>,
    presentation_fields: &'a HashMap<String, HashSet<String>>,
}

#[derive(Clone, Debug)]
enum ValueFact {
    Unknown,
    Localized,
    Literal(Span, String),
    Object(HashMap<String, ValueFact>),
}

impl<'a> UiStringVisitor<'a> {
    fn new(
        source: &'a str,
        wrapper_parameters: &'a HashMap<String, HashSet<usize>>,
        presentation_fields: &'a HashMap<String, HashSet<String>>,
    ) -> Self {
        Self {
            lines: source.lines().collect(),
            diagnostics: Vec::new(),
            bindings: vec![HashMap::new()],
            wrapper_parameters,
            presentation_fields,
        }
    }

    fn report(&mut self, span: Span, literal: String, sink: String) {
        if literal.is_empty() {
            return;
        }
        let start = span.start();
        if self.suppressed(start.line) {
            return;
        }
        self.diagnostics.push(Diagnostic {
            line: start.line,
            column: start.column + 1,
            literal,
            sink,
        });
    }

    fn suppressed(&self, one_based_line: usize) -> bool {
        [one_based_line.checked_sub(1), one_based_line.checked_sub(2)]
            .into_iter()
            .flatten()
            .filter_map(|index| self.lines.get(index))
            .any(|line| {
                line.split_once(SUPPRESSION)
                    .is_some_and(|(_, reason)| !reason.trim().is_empty())
            })
    }

    fn fact(&self, expression: &Expr) -> ValueFact {
        match expression {
            Expr::Lit(expression) => match &expression.lit {
                Lit::Str(literal) => ValueFact::Literal(literal.span(), literal.value()),
                _ => ValueFact::Unknown,
            },
            Expr::Path(expression) if expression.path.segments.len() == 1 => self
                .bindings
                .iter()
                .rev()
                .find_map(|scope| scope.get(&expression.path.segments[0].ident.to_string()))
                .cloned()
                .unwrap_or(ValueFact::Unknown),
            Expr::Struct(expression) => {
                let Some(name) = expression
                    .path
                    .segments
                    .last()
                    .map(|part| part.ident.to_string())
                else {
                    return ValueFact::Unknown;
                };
                let Some(visible) = self.presentation_fields.get(&name) else {
                    return ValueFact::Unknown;
                };
                ValueFact::Object(
                    expression
                        .fields
                        .iter()
                        .filter_map(|field| match &field.member {
                            Member::Named(name) if visible.contains(&name.to_string()) => {
                                Some((name.to_string(), self.fact(&field.expr)))
                            }
                            _ => None,
                        })
                        .collect(),
                )
            }
            Expr::Field(expression) => match (self.fact(&expression.base), &expression.member) {
                (ValueFact::Object(fields), Member::Named(name)) => fields
                    .get(&name.to_string())
                    .cloned()
                    .unwrap_or(ValueFact::Unknown),
                _ => ValueFact::Unknown,
            },
            Expr::Reference(expression) => self.fact(&expression.expr),
            Expr::Paren(expression) => self.fact(&expression.expr),
            Expr::Group(expression) => self.fact(&expression.expr),
            Expr::MethodCall(expression)
                if matches!(
                    expression.method.to_string().as_str(),
                    "text" | "format" | "number" | "label"
                ) =>
            {
                ValueFact::Localized
            }
            Expr::MethodCall(expression)
                if matches!(
                    expression.method.to_string().as_str(),
                    "into" | "to_owned" | "to_string" | "as_ref"
                ) =>
            {
                self.fact(&expression.receiver)
            }
            Expr::Macro(expression) => macro_literal(expression)
                .map(|(span, value)| ValueFact::Literal(span, value))
                .unwrap_or(ValueFact::Unknown),
            _ => ValueFact::Unknown,
        }
    }

    fn check(&mut self, expression: &Expr, sink: String) {
        if let ValueFact::Literal(span, literal) = self.fact(expression) {
            self.report(span, literal, sink);
        }
    }
}

impl<'ast> Visit<'ast> for UiStringVisitor<'_> {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref() {
            let path = function
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if let Some((argument, sink)) = call_sink(&path)
                && let Some(expression) = expression.args.iter().nth(argument)
            {
                self.check(expression, sink);
            } else if let Some(name) = path.last()
                && let Some(parameters) = self.wrapper_parameters.get(name)
            {
                for parameter in parameters {
                    if let Some(argument) = expression.args.iter().nth(*parameter) {
                        self.check(argument, format!("{name} wrapper"));
                    }
                }
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast ExprMethodCall) {
        let method = expression.method.to_string();
        if matches!(method.as_str(), "set_title" | "with_title")
            && let Some(argument) = expression.args.first()
        {
            self.check(argument, method);
        }
        visit::visit_expr_method_call(self, expression);
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if !is_test_code(&item.attrs) {
            visit::visit_item_fn(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if !is_test_code(&item.attrs) {
            visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if !is_test_code(&item.attrs) {
            visit::visit_item_mod(self, item);
        }
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        visit::visit_local(self, local);
        if let Pat::Ident(binding) = &local.pat
            && binding.by_ref.is_none()
            && binding.mutability.is_none()
            && binding.subpat.is_none()
            && let Some(initializer) = &local.init
        {
            let fact = self.fact(&initializer.expr);
            if let Some(scope) = self.bindings.last_mut() {
                scope.insert(binding.ident.to_string(), fact);
            }
        }
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.bindings.push(HashMap::new());
        visit::visit_block(self, block);
        self.bindings.pop();
    }
}

fn call_sink(path: &[String]) -> Option<(usize, String)> {
    let final_segment = path.last()?.as_str();
    if final_segment == "text_buffer" {
        return Some((1, "text_buffer".into()));
    }
    if final_segment != "new" || path.len() < 2 {
        return None;
    }
    let owner = path[path.len() - 2].as_str();
    let argument = match owner {
        "Text" | "ButtonLabel" | "Header" => 0,
        "Button" | "UiButton" | "RadioButton" => 1,
        _ => return None,
    };
    Some((argument, format!("{owner}::new")))
}

fn visible_literal(expression: &Expr) -> Option<(Span, String)> {
    match expression {
        Expr::Lit(expression) => match &expression.lit {
            Lit::Str(literal) => Some((literal.span(), literal.value())),
            _ => None,
        },
        Expr::Reference(expression) => visible_literal(&expression.expr),
        Expr::Paren(expression) => visible_literal(&expression.expr),
        Expr::Group(expression) => visible_literal(&expression.expr),
        Expr::MethodCall(expression)
            if matches!(
                expression.method.to_string().as_str(),
                "into" | "to_owned" | "to_string" | "as_ref"
            ) =>
        {
            visible_literal(&expression.receiver)
        }
        Expr::Macro(expression) => macro_literal(expression),
        _ => None,
    }
}

fn macro_literal(expression: &ExprMacro) -> Option<(Span, String)> {
    if !matches!(
        expression
            .mac
            .path
            .segments
            .last()?
            .ident
            .to_string()
            .as_str(),
        "format" | "concat"
    ) {
        return None;
    }
    first_macro_string(&expression.mac)
}

fn first_macro_string(mac: &Macro) -> Option<(Span, String)> {
    let expressions = mac
        .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        .ok()?;
    visible_literal(expressions.first()?)
}

fn is_test_code(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || (attribute.path().is_ident("cfg")
                && matches!(
                    &attribute.meta,
                    syn::Meta::List(list) if list.tokens.to_string() == "test"
                ))
    })
}

fn presentation_fields(file: &syn::File) -> HashMap<String, HashSet<String>> {
    #[derive(Default)]
    struct Collector {
        fields: HashMap<String, HashSet<String>>,
    }
    impl<'ast> Visit<'ast> for Collector {
        fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
            let visible = item
                .fields
                .iter()
                .filter(|field| {
                    field.attrs.iter().any(|attribute| {
                        attribute.path().is_ident("nickel_i18n")
                            && matches!(
                                &attribute.meta,
                                syn::Meta::List(list) if list.tokens.to_string() == "presentation"
                            )
                    })
                })
                .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
                .collect::<HashSet<_>>();
            if !visible.is_empty() {
                self.fields.insert(item.ident.to_string(), visible);
            }
            visit::visit_item_struct(self, item);
        }
    }
    let mut collector = Collector::default();
    collector.visit_file(file);
    collector.fields
}

#[derive(Clone)]
struct FunctionFlow<'a> {
    name: String,
    parameters: HashMap<String, usize>,
    body: &'a syn::Block,
}

#[derive(Default)]
struct FunctionCollector<'a> {
    functions: Vec<FunctionFlow<'a>>,
}

impl<'ast> Visit<'ast> for FunctionCollector<'ast> {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if is_test_code(&item.attrs) {
            return;
        }
        let parameters = item
            .sig
            .inputs
            .iter()
            .filter_map(|argument| match argument {
                FnArg::Typed(argument) => match argument.pat.as_ref() {
                    Pat::Ident(binding) => Some(binding.ident.to_string()),
                    _ => None,
                },
                FnArg::Receiver(_) => None,
            })
            .enumerate()
            .map(|(index, name)| (name, index))
            .collect();
        self.functions.push(FunctionFlow {
            name: item.sig.ident.to_string(),
            parameters,
            body: &item.block,
        });
        visit::visit_block(self, &item.block);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if is_test_code(&item.attrs) {
            return;
        }
        let parameters = item
            .sig
            .inputs
            .iter()
            .filter_map(|argument| match argument {
                FnArg::Typed(argument) => match argument.pat.as_ref() {
                    Pat::Ident(binding) => Some(binding.ident.to_string()),
                    _ => None,
                },
                FnArg::Receiver(_) => None,
            })
            .enumerate()
            .map(|(index, name)| (name, index))
            .collect();
        self.functions.push(FunctionFlow {
            name: item.sig.ident.to_string(),
            parameters,
            body: &item.block,
        });
        visit::visit_block(self, &item.block);
    }
}

fn expression_parameters(expression: &Expr, parameters: &HashMap<String, usize>) -> HashSet<usize> {
    struct Dependencies<'a> {
        parameters: &'a HashMap<String, usize>,
        found: HashSet<usize>,
    }
    impl<'ast> Visit<'ast> for Dependencies<'_> {
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            if path.path.segments.len() == 1
                && let Some(index) = self
                    .parameters
                    .get(&path.path.segments[0].ident.to_string())
            {
                self.found.insert(*index);
            }
            visit::visit_expr_path(self, path);
        }
    }
    let mut dependencies = Dependencies {
        parameters,
        found: HashSet::new(),
    };
    dependencies.visit_expr(expression);
    dependencies.found
}

fn wrapper_parameters(file: &syn::File) -> HashMap<String, HashSet<usize>> {
    let mut collector = FunctionCollector::default();
    collector.visit_file(file);
    let function_names = collector
        .functions
        .iter()
        .map(|function| function.name.clone())
        .collect::<HashSet<_>>();
    let mut summaries = HashMap::<String, HashSet<usize>>::new();

    // Iteration over the finite parameter sets is a monotone fixed point. It
    // converges for recursive wrapper groups as well as acyclic call chains.
    loop {
        let mut changed = false;
        for function in &collector.functions {
            struct FlowVisitor<'a> {
                parameters: &'a HashMap<String, usize>,
                summaries: &'a HashMap<String, HashSet<usize>>,
                function_names: &'a HashSet<String>,
                found: HashSet<usize>,
            }
            impl<'ast> Visit<'ast> for FlowVisitor<'_> {
                fn visit_expr_call(&mut self, call: &'ast ExprCall) {
                    if let Expr::Path(path) = call.func.as_ref() {
                        let segments = path
                            .path
                            .segments
                            .iter()
                            .map(|segment| segment.ident.to_string())
                            .collect::<Vec<_>>();
                        let sensitive = call_sink(&segments)
                            .map(|(index, _)| HashSet::from([index]))
                            .or_else(|| {
                                let name = segments.last()?;
                                self.function_names
                                    .contains(name)
                                    .then(|| self.summaries.get(name).cloned().unwrap_or_default())
                            })
                            .unwrap_or_default();
                        for index in sensitive {
                            if let Some(argument) = call.args.iter().nth(index) {
                                self.found
                                    .extend(expression_parameters(argument, self.parameters));
                            }
                        }
                    }
                    visit::visit_expr_call(self, call);
                }

                fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
                    if matches!(call.method.to_string().as_str(), "set_title" | "with_title")
                        && let Some(argument) = call.args.first()
                    {
                        self.found
                            .extend(expression_parameters(argument, self.parameters));
                    }
                    visit::visit_expr_method_call(self, call);
                }
            }
            let found = {
                let mut visitor = FlowVisitor {
                    parameters: &function.parameters,
                    summaries: &summaries,
                    function_names: &function_names,
                    found: HashSet::new(),
                };
                visitor.visit_block(function.body);
                visitor.found
            };
            let summary = summaries.entry(function.name.clone()).or_default();
            let old_len = summary.len();
            summary.extend(found);
            changed |= summary.len() != old_len;
        }
        if !changed {
            break;
        }
    }
    summaries.retain(|_, parameters| !parameters.is_empty());
    summaries
}

fn rust_files(path: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path.to_owned());
        }
        return Ok(());
    }
    if !path.is_dir()
        || path
            .file_name()
            .is_some_and(|name| matches!(name.to_str(), Some("target" | ".git")))
    {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        rust_files(&entry?.path(), output)?;
    }
    Ok(())
}

fn lint_file(path: &Path) -> Result<Vec<Diagnostic>, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let syntax =
        syn::parse_file(&source).map_err(|error| format!("{}: {error}", path.display()))?;
    let wrappers = wrapper_parameters(&syntax);
    let fields = presentation_fields(&syntax);
    let mut visitor = UiStringVisitor::new(&source, &wrappers, &fields);
    visitor.visit_file(&syntax);
    Ok(visitor.diagnostics)
}

fn main() -> ExitCode {
    let inputs = env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let inputs = if inputs.is_empty() {
        vec![PathBuf::from("crates")]
    } else {
        inputs
    };
    let mut files = Vec::new();
    for input in &inputs {
        if let Err(error) = rust_files(input, &mut files) {
            eprintln!("{}: {error}", input.display());
            return ExitCode::from(2);
        }
    }
    files.sort();
    let mut violation_count = 0;
    for file in files {
        match lint_file(&file) {
            Ok(diagnostics) => {
                for diagnostic in diagnostics {
                    violation_count += 1;
                    eprintln!(
                        "{}:{}:{}: {CODE} hardcoded user-interface string {:?} passed to {}",
                        file.display(),
                        diagnostic.line,
                        diagnostic.column,
                        diagnostic.literal,
                        diagnostic.sink,
                    );
                }
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
        }
    }
    if violation_count == 0 {
        ExitCode::SUCCESS
    } else {
        eprintln!("{violation_count} localization violation(s)");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use syn::visit::Visit;

    use super::UiStringVisitor;

    fn lint(source: &str) -> Vec<super::Diagnostic> {
        let file = syn::parse_file(source).expect("fixture parses");
        let wrappers = super::wrapper_parameters(&file);
        let fields = super::presentation_fields(&file);
        let mut visitor = UiStringVisitor::new(source, &wrappers, &fields);
        visitor.visit_file(&file);
        visitor.diagnostics
    }

    #[test]
    fn catches_literals_and_format_templates_at_ui_sinks() {
        let diagnostics = lint(
            r#"
fn render(name: &str) {
    let _ = Text::new("Hello");
    let _ = Button::new("open", format!("Open {name}"));
}
"#,
        );
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.literal.as_str(), diagnostic.sink.as_str()))
                .collect::<Vec<_>>(),
            [("Hello", "Text::new"), ("Open {name}", "Button::new")]
        );
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.line > 0));
    }

    #[test]
    fn accepts_localized_and_dynamic_values() {
        assert!(
            lint(
                r#"
fn render(localizer: &Localizer, name: String) {
    let _ = Text::new(localizer.text("hello"));
    let _ = Text::new(name);
}
"#
            )
            .is_empty()
        );
    }

    #[test]
    fn reasoned_suppression_is_local() {
        assert!(
            lint(
                r#"
fn render() {
    // nickel-i18n-lint: allow icon-only control
    let _ = Text::new("⌁");
}
"#
            )
            .is_empty()
        );
        assert!(
            lint(
                r#"
fn render() {
    let _ = Text::new("⌁"); // nickel-i18n-lint: allow icon-only control
}
"#
            )
            .is_empty()
        );
        assert_eq!(
            lint(
                r#"
fn render() {
    // nickel-i18n-lint: allow
    let _ = Text::new("Unexplained");
}
"#
            )
            .len(),
            1
        );
    }

    #[test]
    fn test_code_is_ignored() {
        assert!(
            lint(
                r#"
#[cfg(test)]
mod tests {
    fn fixture() { let _ = Text::new("Fixture"); }
}
"#
            )
            .is_empty()
        );
    }

    #[test]
    fn follows_immutable_local_bindings() {
        let diagnostics = lint(
            r#"
fn render() {
    let label = "Open";
    let converted = label.to_owned();
    Button::new(Message::Open, converted);
}
"#,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].literal, "Open");
    }

    #[test]
    fn follows_recursive_wrapper_functions() {
        let diagnostics = lint(
            r#"
fn outer(label: String) { inner(label) }
fn inner(label: String) { either(label) }
fn either(label: String) {
    if ready() { outer(label) } else { Text::new(label) }
}
fn render() { outer("Untranslated".into()) }
"#,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].literal, "Untranslated");
        assert_eq!(diagnostics[0].sink, "outer wrapper");
    }

    #[test]
    fn localized_values_remain_accepted_through_bindings_and_wrappers() {
        assert!(
            lint(
                r#"
fn heading(label: String) { Header::new(label); }
fn render(localizer: &Localizer) {
    let label = localizer.text("heading-key");
    heading(label);
}
"#,
            )
            .is_empty()
        );
    }

    #[test]
    fn scans_files_and_directories_but_skips_build_and_vcs_trees() {
        let directory = tempfile::tempdir().expect("temporary fixture directory");
        fs::create_dir_all(directory.path().join("src/nested")).unwrap();
        fs::create_dir_all(directory.path().join("target/generated")).unwrap();
        fs::create_dir_all(directory.path().join(".git/hooks")).unwrap();
        fs::write(directory.path().join("src/lib.rs"), "fn main() {}\n").unwrap();
        fs::write(
            directory.path().join("src/nested/view.rs"),
            "fn view() {}\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("target/generated/view.rs"),
            "ignored\n",
        )
        .unwrap();
        fs::write(directory.path().join(".git/hooks/check.rs"), "ignored\n").unwrap();

        let mut files = Vec::new();
        super::rust_files(directory.path(), &mut files).unwrap();
        files.sort();
        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .all(|path| path.starts_with(directory.path().join("src")))
        );

        let mut explicit = Vec::new();
        super::rust_files(&directory.path().join("src/lib.rs"), &mut explicit).unwrap();
        assert_eq!(explicit, [directory.path().join("src/lib.rs")]);
    }

    #[test]
    fn mutable_bindings_are_unknown_after_initialization() {
        assert!(
            lint(
                r#"
fn render(name: String) {
    let mut label = "Initial";
    label = name;
    Text::new(label);
}
"#,
            )
            .is_empty()
        );
    }

    #[test]
    fn follows_explicitly_annotated_presentation_fields_only() {
        let diagnostics = lint(
            r#"
struct Card {
    #[nickel_i18n(presentation)]
    heading: String,
    protocol_token: String,
}
fn render() {
    let card = Card {
        heading: "Account".into(),
        protocol_token: "internal-v1".into(),
    };
    Header::new(card.heading);
    Text::new(card.protocol_token);
}
"#,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].literal, "Account");
    }
}
