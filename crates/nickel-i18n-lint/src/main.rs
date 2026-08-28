use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use proc_macro2::Span;
use syn::{
    Attribute, Expr, ExprCall, ExprMacro, ExprMethodCall, ImplItemFn, ItemFn, ItemMod, Lit, Macro,
    Token,
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
}

impl<'a> UiStringVisitor<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            lines: source.lines().collect(),
            diagnostics: Vec::new(),
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
                && let Some((span, literal)) = visible_literal(expression)
            {
                self.report(span, literal, sink);
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast ExprMethodCall) {
        let method = expression.method.to_string();
        if matches!(method.as_str(), "set_title" | "with_title")
            && let Some(argument) = expression.args.first()
            && let Some((span, literal)) = visible_literal(argument)
        {
            self.report(span, literal, method);
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
    let mut visitor = UiStringVisitor::new(&source);
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
    use syn::visit::Visit;

    use super::UiStringVisitor;

    fn lint(source: &str) -> Vec<super::Diagnostic> {
        let file = syn::parse_file(source).expect("fixture parses");
        let mut visitor = UiStringVisitor::new(source);
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
}
