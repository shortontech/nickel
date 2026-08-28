use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    Error, Expr, FnArg, Ident, ItemFn, LitStr, Pat, Result, Token, Type, braced,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

struct ViewInput(Vec<Node>);

enum Node {
    Element(ElementNode),
    Expression(Expr),
    Text(LitStr),
}

struct ElementNode {
    tag: Ident,
    attributes: Vec<Attribute>,
    children: Vec<Node>,
}

struct Attribute {
    name: Ident,
    value: Option<Expr>,
}

impl Parse for ViewInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut nodes = Vec::new();
        while !input.is_empty() {
            nodes.push(parse_node(input)?);
        }
        Ok(Self(nodes))
    }
}

fn parse_node(input: ParseStream<'_>) -> Result<Node> {
    if input.peek(Token![<]) {
        return Ok(Node::Element(parse_element(input)?));
    }
    if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);
        return Ok(Node::Expression(content.parse()?));
    }
    if input.peek(LitStr) {
        return Ok(Node::Text(input.parse()?));
    }
    Err(input.error("expected a component, a braced Rust expression, or a string child"))
}

fn parse_element(input: ParseStream<'_>) -> Result<ElementNode> {
    input.parse::<Token![<]>()?;
    let fragment = input.peek(Token![>]);
    let tag: Ident = if fragment {
        Ident::new("Fragment", input.span())
    } else {
        input.parse()?
    };
    let mut attributes = Vec::new();
    while !input.peek(Token![>]) && !input.peek(Token![/]) {
        let name: Ident = input.parse()?;
        let value = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let content;
            braced!(content in input);
            Some(content.parse()?)
        } else {
            None
        };
        if attributes
            .iter()
            .any(|attribute: &Attribute| attribute.name == name)
        {
            return Err(Error::new(
                name.span(),
                format!("duplicate property `{name}`"),
            ));
        }
        attributes.push(Attribute { name, value });
    }
    if input.peek(Token![/]) {
        input.parse::<Token![/]>()?;
        input.parse::<Token![>]>()?;
        return Ok(ElementNode {
            tag,
            attributes,
            children: Vec::new(),
        });
    }
    input.parse::<Token![>]>()?;
    let mut children = Vec::new();
    loop {
        if input.peek(Token![<]) && input.peek2(Token![/]) {
            input.parse::<Token![<]>()?;
            input.parse::<Token![/]>()?;
            let close: Ident = if fragment && input.peek(Token![>]) {
                Ident::new("Fragment", input.span())
            } else {
                input.parse()?
            };
            input.parse::<Token![>]>()?;
            if close != tag {
                return Err(Error::new(
                    close.span(),
                    format!("expected closing tag `</{tag}>`"),
                ));
            }
            break;
        }
        if input.is_empty() {
            return Err(Error::new(
                tag.span(),
                format!("missing closing tag `</{tag}>`"),
            ));
        }
        children.push(parse_node(input)?);
    }
    Ok(ElementNode {
        tag,
        attributes,
        children,
    })
}

#[proc_macro]
pub fn ui(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ViewInput);
    match expand_root(input.0) {
        Ok(tokens) => quote! { ::nickel_ui::Component::into_element(#tokens) }.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_root(nodes: Vec<Node>) -> Result<TokenStream2> {
    if nodes.len() == 1 {
        expand_node(nodes.into_iter().next().expect("one node"))
    } else {
        let children = nodes
            .into_iter()
            .map(expand_node)
            .collect::<Result<Vec<_>>>()?;
        Ok(quote! { ::nickel_ui::Fragment::new().children([#(#children),*]) })
    }
}

fn expand_node(node: Node) -> Result<TokenStream2> {
    match node {
        Node::Element(element) => expand_element(element),
        Node::Expression(expression) => Ok(quote! { #expression }),
        Node::Text(text) => Ok(quote! { ::nickel_ui::Text::new(#text) }),
    }
}

fn take_attribute(attributes: &mut Vec<Attribute>, name: &str) -> Option<Attribute> {
    attributes
        .iter()
        .position(|attribute| attribute.name == name)
        .map(|index| attributes.remove(index))
}

fn required_value(attributes: &mut Vec<Attribute>, name: &str, span: Span) -> Result<Expr> {
    let attribute = take_attribute(attributes, name)
        .ok_or_else(|| Error::new(span, format!("missing required property `{name}`")))?;
    attribute.value.ok_or_else(|| {
        Error::new(
            attribute.name.span(),
            format!("property `{name}` requires a braced Rust value"),
        )
    })
}

fn expand_element(mut element: ElementNode) -> Result<TokenStream2> {
    let tag_name = element.tag.to_string();
    let tag = &element.tag;
    let mut expression = match tag_name.as_str() {
        "Text" => {
            if element.children.len() != 1 {
                return Err(Error::new(
                    tag.span(),
                    "`Text` requires exactly one child value",
                ));
            }
            let value = expand_node(element.children.remove(0))?;
            quote! { ::nickel_ui::Text::new(#value) }
        }
        "Header" => {
            let title = if let Some(attribute) = take_attribute(&mut element.attributes, "title") {
                if !element.children.is_empty() {
                    return Err(Error::new(
                        tag.span(),
                        "`Header` accepts either `title` or one title child, not both",
                    ));
                }
                let value = attribute.value.ok_or_else(|| {
                    Error::new(
                        attribute.name.span(),
                        "`title` requires a braced Rust value",
                    )
                })?;
                quote! { #value }
            } else {
                if element.children.len() != 1 {
                    return Err(Error::new(
                        tag.span(),
                        "`Header` requires `title` or exactly one title child",
                    ));
                }
                expand_raw_child(element.children.remove(0))?
            };
            quote! { ::nickel_ui::Header::new(#title) }
        }
        "Button" => {
            let message = required_value(&mut element.attributes, "on_press", tag.span())?;
            if element.children.len() != 1 {
                return Err(Error::new(
                    tag.span(),
                    "`Button` requires exactly one label child",
                ));
            }
            let label = expand_raw_child(element.children.remove(0))?;
            quote! { ::nickel_ui::Button::new(#message, #label) }
        }
        "Menu" => {
            let toggle = required_value(&mut element.attributes, "on_toggle", tag.span())?;
            let label = required_value(&mut element.attributes, "label", tag.span())?;
            let items = element
                .children
                .drain(..)
                .map(|child| match child {
                    Node::Element(mut item) if item.tag == "MenuItem" => {
                        let item_label =
                            required_value(&mut item.attributes, "label", item.tag.span())?;
                        if take_attribute(&mut item.attributes, "disabled").is_some() {
                            Ok(quote! { ::nickel_ui::MenuItem::disabled(#item_label) })
                        } else {
                            let message =
                                required_value(&mut item.attributes, "on_press", item.tag.span())?;
                            Ok(quote! { ::nickel_ui::MenuItem::new(#item_label, #message) })
                        }
                    }
                    _ => Err(Error::new(
                        tag.span(),
                        "`Menu` children must be `MenuItem` elements",
                    )),
                })
                .collect::<Result<Vec<_>>>()?;
            quote! { ::nickel_ui::Menu::new(#toggle, #label, [#(#items),*]) }
        }
        "Slider" => {
            let value = required_value(&mut element.attributes, "value", tag.span())?;
            let mapper = required_value(&mut element.attributes, "on_change", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(tag.span(), "`Slider` does not accept children"));
            }
            quote! { ::nickel_ui::Slider::on_change(#mapper, #value) }
        }
        "TextField" => {
            let value = required_value(&mut element.attributes, "value", tag.span())?;
            let mapper = required_value(&mut element.attributes, "on_change", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(
                    tag.span(),
                    "`TextField` does not accept children",
                ));
            }
            quote! { ::nickel_ui::TextField::on_change(#value, #mapper) }
        }
        "RadioButton" => {
            let message = required_value(&mut element.attributes, "on_press", tag.span())?;
            let label = required_value(&mut element.attributes, "label", tag.span())?;
            let selected = required_value(&mut element.attributes, "selected", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(
                    tag.span(),
                    "`RadioButton` does not accept children",
                ));
            }
            quote! { ::nickel_ui::RadioButton::new(#message, #label, #selected) }
        }
        "Dropdown" => {
            let toggle = required_value(&mut element.attributes, "on_toggle", tag.span())?;
            let selected = required_value(&mut element.attributes, "selected", tag.span())?;
            let options = required_value(&mut element.attributes, "options", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(
                    tag.span(),
                    "`Dropdown` does not accept children",
                ));
            }
            quote! { ::nickel_ui::Dropdown::new(#toggle, #selected, #options) }
        }
        "VerticalScroll" => {
            let message = required_value(&mut element.attributes, "on_scroll", tag.span())?;
            let offset = take_attribute(&mut element.attributes, "offset")
                .and_then(|attribute| attribute.value)
                .map_or_else(|| quote! { 0.0 }, |value| quote! { #value });
            quote! { ::nickel_ui::VerticalScroll::new(#message, #offset) }
        }
        "FileGrid" => {
            let minimum = required_value(&mut element.attributes, "min_width", tag.span())?;
            quote! { ::nickel_ui::FileGrid::auto_fit(#minimum) }
        }
        "FileGridItem" => {
            let message = required_value(&mut element.attributes, "on_press", tag.span())?;
            let label = required_value(&mut element.attributes, "label", tag.span())?;
            let asset_id = required_value(&mut element.attributes, "asset_id", tag.span())?;
            let image = required_value(&mut element.attributes, "image", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(
                    tag.span(),
                    "`FileGridItem` does not accept children",
                ));
            }
            quote! { ::nickel_ui::FileGridItem::new(#message, #label, #asset_id, #image) }
        }
        "Sidebar" => {
            let width = required_value(&mut element.attributes, "width", tag.span())?;
            quote! { ::nickel_ui::Sidebar::new(#width) }
        }
        "HorizontalRule" => {
            let color = required_value(&mut element.attributes, "color", tag.span())?;
            quote! { ::nickel_ui::HorizontalRule::new(#color) }
        }
        "SidebarSection" => {
            let title = required_value(&mut element.attributes, "title", tag.span())?;
            let color = required_value(&mut element.attributes, "color", tag.span())?;
            quote! { ::nickel_ui::SidebarSection::new(#title, #color) }
        }
        "SidebarFolder" => {
            let toggle = required_value(&mut element.attributes, "on_toggle", tag.span())?;
            let open = required_value(&mut element.attributes, "on_open", tag.span())?;
            let label = required_value(&mut element.attributes, "label", tag.span())?;
            let expanded = required_value(&mut element.attributes, "expanded", tag.span())?;
            let foreground = required_value(&mut element.attributes, "foreground", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(
                    tag.span(),
                    "`SidebarFolder` does not accept children",
                ));
            }
            quote! {
                ::nickel_ui::SidebarFolder::new(#toggle, #open, #label, #expanded, #foreground)
            }
        }
        "ShoulderHints" => {
            let color = required_value(&mut element.attributes, "color", tag.span())?;
            let muted = required_value(&mut element.attributes, "muted", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(
                    tag.span(),
                    "`ShoulderHints` does not accept children",
                ));
            }
            quote! { ::nickel_ui::ShoulderHints::new(#color, #muted) }
        }
        "Image" => {
            let asset_id = required_value(&mut element.attributes, "asset_id", tag.span())?;
            let image = required_value(&mut element.attributes, "image", tag.span())?;
            if !element.children.is_empty() {
                return Err(Error::new(tag.span(), "`Image` does not accept children"));
            }
            quote! { ::nickel_ui::Image::new(#asset_id, #image) }
        }
        "Column" | "Row" | "Container" | "Grid" | "Fragment" | "MenuBar" => {
            quote! { ::nickel_ui::#tag::new() }
        }
        "Spacer" => quote! { ::nickel_ui::Spacer::new() },
        _ => {
            let mut identity = Vec::new();
            let mut properties = Vec::new();
            for attribute in element.attributes.drain(..) {
                if attribute.name == "key" || attribute.name == "id" {
                    identity.push(attribute);
                } else {
                    properties.push(attribute);
                }
            }
            element.attributes = identity;
            let mut properties = properties
                .into_iter()
                .map(|attribute| {
                    let name = attribute.name;
                    let value = attribute.value.ok_or_else(|| {
                        Error::new(name.span(), "component properties require values")
                    })?;
                    Ok(quote! { #name = { #value } })
                })
                .collect::<Result<Vec<_>>>()?;
            if element.children.len() == 1 {
                let child = element.children.remove(0);
                let is_iterator = matches!(
                    &child,
                    Node::Expression(Expr::MethodCall(call))
                        if matches!(
                            call.method.to_string().as_str(),
                            "map" | "flat_map" | "filter_map" | "into_iter"
                        )
                );
                let child = expand_node(child)?;
                if is_iterator {
                    properties.push(quote! { children = { #child } });
                } else {
                    properties.push(quote! { child = { #child } });
                }
            } else if !element.children.is_empty() {
                let children = element
                    .children
                    .drain(..)
                    .map(expand_node)
                    .collect::<Result<Vec<_>>>()?;
                properties.push(quote! { children = { [#(#children),*] } });
            }
            return Ok(quote! { #tag! { #(#properties),* } });
        }
    };

    for attribute in element.attributes {
        let name = attribute.name;
        expression = if name == "key" || name == "id" {
            let value = attribute
                .value
                .ok_or_else(|| Error::new(name.span(), "identity requires a value"))?;
            quote! { (#expression).id(#value) }
        } else if name == "fill" {
            if attribute.value.is_some() {
                return Err(Error::new(name.span(), "`fill` is a boolean property"));
            }
            quote! { (#expression).grow(1.0) }
        } else if name == "on_press" {
            let value = attribute
                .value
                .ok_or_else(|| Error::new(name.span(), "`on_press` requires a message"))?;
            quote! { (#expression).message(#value) }
        } else if name == "border" {
            let value = attribute
                .value
                .ok_or_else(|| Error::new(name.span(), "`border` requires a typed border value"))?;
            quote! { (#expression).border_value(#value) }
        } else if let Some(value) = attribute.value {
            quote! { (#expression).#name(#value) }
        } else {
            quote! { (#expression).#name() }
        };
    }

    for child in element.children {
        let is_iterator = matches!(
            &child,
            Node::Expression(Expr::MethodCall(call))
                if matches!(
                    call.method.to_string().as_str(),
                    "map" | "flat_map" | "filter_map" | "into_iter"
                )
        );
        let child = expand_node(child)?;
        expression = if is_iterator {
            quote! { (#expression).children(#child) }
        } else {
            quote! { (#expression).child(#child) }
        };
    }
    let component = LitStr::new(&tag_name, tag.span());
    Ok(quote! {
        ::nickel_ui::Component::into_element(#expression).with_source(
            ::nickel_ui::SourceLocation::new(#component, file!(), line!(), column!())
        )
    })
}

fn expand_raw_child(node: Node) -> Result<TokenStream2> {
    match node {
        Node::Expression(expression) => Ok(quote! { #expression }),
        Node::Text(text) => Ok(quote! { #text }),
        Node::Element(element) => Err(Error::new(
            element.tag.span(),
            "this property expects a value rather than a nested component",
        )),
    }
}

#[proc_macro]
pub fn id(input: TokenStream) -> TokenStream {
    let identifier = parse_macro_input!(input as Ident);
    let value = LitStr::new(&identifier.to_string().replace('_', "-"), identifier.span());
    quote! { ::nickel_ui::UiId::from(#value) }.into()
}

#[proc_macro_attribute]
pub fn component(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);
    match expand_component(function) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_component(mut function: ItemFn) -> Result<TokenStream2> {
    let name = function.sig.ident.clone();
    if function.sig.inputs.len() > 7 {
        return Err(Error::new_spanned(
            &function.sig.inputs,
            "declarative components support at most seven properties",
        ));
    }
    let mut properties = Vec::new();
    for argument in &function.sig.inputs {
        let FnArg::Typed(argument) = argument else {
            return Err(Error::new_spanned(
                argument,
                "components cannot have a `self` receiver",
            ));
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            return Err(Error::new_spanned(
                &argument.pat,
                "component properties must use simple identifiers",
            ));
        };
        properties.push((pattern.ident.clone(), is_option(&argument.ty)));
    }
    function
        .attrs
        .push(syn::parse_quote!(#[allow(non_snake_case)]));

    let optional_positions = properties
        .iter()
        .enumerate()
        .filter_map(|(index, (_, optional))| optional.then_some(index))
        .collect::<Vec<_>>();
    let mut arms = Vec::new();
    for mask in 0..(1usize << optional_positions.len()) {
        let included = (0..properties.len())
            .filter(|index| {
                optional_positions
                    .iter()
                    .position(|optional| optional == index)
                    .is_none_or(|bit| mask & (1 << bit) != 0)
            })
            .collect::<Vec<_>>();
        for order in permutations(&included) {
            let pattern = order.iter().map(|index| {
                let property = &properties[*index].0;
                let variable = format_ident!("__nickel_{property}");
                quote! { #property = { $ #variable:expr } }
            });
            let arguments = properties
                .iter()
                .enumerate()
                .map(|(index, (property, optional))| {
                    let variable = format_ident!("__nickel_{property}");
                    if *optional {
                        if included.contains(&index) {
                            quote! { ::core::option::Option::Some($ #variable) }
                        } else {
                            quote! { ::core::option::Option::None }
                        }
                    } else {
                        quote! { $ #variable }
                    }
                });
            arms.push(quote! {
                (#(#pattern),* $(,)?) => { #name(#(#arguments),*) };
            });
        }
    }
    Ok(quote! {
        #function
        #[allow(unused_macros)]
        macro_rules! #name {
            #(#arms)*
        }
    })
}

fn is_option(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "Option")
    )
}

fn permutations(values: &[usize]) -> Vec<Vec<usize>> {
    if values.is_empty() {
        return vec![Vec::new()];
    }
    let mut output = Vec::new();
    for index in 0..values.len() {
        let value = values[index];
        let mut remaining = values.to_vec();
        remaining.remove(index);
        for mut suffix in permutations(&remaining) {
            suffix.insert(0, value);
            output.push(suffix);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{ViewInput, expand_component, expand_root, permutations};
    use quote::quote;

    fn parse(input: proc_macro2::TokenStream) -> ViewInput {
        syn::parse2(input).expect("declarative input should parse")
    }

    fn parse_error(input: proc_macro2::TokenStream) -> String {
        match syn::parse2::<ViewInput>(input) {
            Ok(_) => panic!("declarative input unexpectedly parsed"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn parser_accepts_nested_components_expressions_and_text_children() {
        let input = parse(quote! {
            <Column id={"settings"}>
                "Settings"
                {items.into_iter().map(render_item)}
                <Button on_press={Message::Save}> "Save" </Button>
            </Column>
        });

        assert_eq!(input.0.len(), 1);
        let expanded = expand_root(input.0)
            .expect("valid component tree should expand")
            .to_string();
        assert!(expanded.contains("SourceLocation"));
        assert!(expanded.contains("children"));
        assert!(expanded.contains("Message :: Save"));
    }

    #[test]
    fn parser_rejects_duplicate_properties_mismatched_closers_and_unclosed_elements() {
        let duplicate = parse_error(quote! {
            <Button on_press={save} on_press={save}> "Save" </Button>
        });
        assert_eq!(duplicate, "duplicate property `on_press`");

        let mismatched = parse_error(quote! { <Column> </Row> });
        assert_eq!(mismatched, "expected closing tag `</Column>`");

        let unclosed = parse_error(quote! { <Column> "content" });
        assert_eq!(unclosed, "missing closing tag `</Column>`");
    }

    #[test]
    fn expansion_rejects_missing_required_values_and_invalid_builtin_children() {
        let missing = parse(quote! { <Button> "Save" </Button> });
        let error = expand_root(missing.0)
            .expect_err("Button without on_press must be rejected")
            .to_string();
        assert_eq!(error, "missing required property `on_press`");

        let child = parse(quote! { <Slider value={value} on_change={change}> "bad" </Slider> });
        let error = expand_root(child.0)
            .expect_err("Slider children must be rejected")
            .to_string();
        assert_eq!(error, "`Slider` does not accept children");
    }

    #[test]
    fn component_expansion_enforces_the_public_property_arity() {
        let function = syn::parse2(quote! {
            fn too_many(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}
        })
        .expect("component fixture should parse");
        let error = expand_component(function)
            .expect_err("components with eight properties must be rejected")
            .to_string();
        assert_eq!(
            error,
            "declarative components support at most seven properties"
        );
    }

    #[test]
    fn optional_component_properties_generate_each_supported_call_shape() {
        let function = syn::parse2(quote! {
            fn panel(title: String, compact: Option<bool>) {}
        })
        .expect("component fixture should parse");
        let expanded = expand_component(function)
            .expect("component with one optional property should expand")
            .to_string();

        assert_eq!(expanded.matches("=>").count(), 3);
        assert!(expanded.contains("Option :: Some"));
        assert!(expanded.contains("Option :: None"));
    }

    #[test]
    fn permutations_preserves_all_orders_without_duplicates() {
        assert_eq!(
            permutations(&[0, 1, 2]),
            vec![
                vec![0, 1, 2],
                vec![0, 2, 1],
                vec![1, 0, 2],
                vec![1, 2, 0],
                vec![2, 0, 1],
                vec![2, 1, 0],
            ]
        );
    }
}
