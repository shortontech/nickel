# nickel-ui

`nickel-ui` is a native Rust UX layer with React-style declarative authoring, typed messages,
intrinsic responsive layout, stable interaction state, and an included SDL presenter. It has no
dependency on the Nickel desktop shell or any other Nickel product crate.

~~~rust
use nickel_ui::prelude::*;

#[derive(Clone)]
enum Message {
    Save,
    SetVolume(f32),
}

fn set_volume(value: f32) -> Message {
    Message::SetVolume(value)
}

fn view(title: &str, volume: f32) -> impl View<Message> {
    ui! {
        <Column gap={12.0} padding={Insets::all(20.0)} fill_width>
            <Text>{title}</Text>
            <Row align_items={Align::Center}>
                <Button id={id!(save)} on_press={Message::Save}>{"Save"}</Button>
                <Spacer fill />
                <Slider value={volume} on_change={set_volume} />
            </Row>
        </Column>
    }
}
~~~

The builder API remains public and supported as an alternative. Declarative and builder calls
produce the same typed component tree and use the same measurement, state, diagnostics, event, and
rendering paths. Its complete API reference is the rustdoc for the component types, `Element`, and
`ComponentBuilderExt`; `examples/builder.rs` is a focused optional example. The default learning
path and application runtime do not require it.

An executable implements `Application` and calls `run(app)`. The runtime owns SDL initialization,
the native window, event and redraw loops, presentation, and the per-window `UiStateStore`; the
application owns only domain state, typed messages, `update`, and `view`. See
`examples/standalone.rs` for the complete counter application.

## Controller navigation

Controller structure is declarative and renderer-owned. Mark shoulder-switchable regions with
`NavigationScope::pane`, and mark nested levels with `NavigationScope::group`. Each scope declares
its traversal, entry, exit, peer, direction, retained-focus, and scroll-owner policy. Stable tree
parentage supplies scope topology; identifier prefixes never do. `ControllerActivate` enters a group or begins editing a slider;
`ControllerBack` exits one level; `ControllerAdjust` changes an active slider by its
`controller_step`. Navigation automatically reveals off-screen semantic targets.

~~~rust,ignore
ui! {
    <Container navigation_scope={NavigationScope::pane(true)}
        navigation_scope_highlight={theme.surfaces.selected}>
        <Container navigation_scope={NavigationScope::group()}
            controller_focus_border={theme.borders.controller_focus}>
            <Slider value={volume} on_change={set_volume} controller_step={0.05}
                controller_focus_border={theme.borders.controller_focus} />
        </Container>
    </Container>
}
~~~

## Semantic visual system

`SemanticTheme` is the product-neutral visual contract. Applications supply light and dark
`semantic token sets`; `ThemePreferences::resolve` combines the stored appearance with platform and
accessibility preferences, and `SemanticTheme::resolve` produces typed surface, border, text,
accent, spacing, radius, sizing, typography, and motion roles. The ordinary accent communicates
product selection; the contrasting controller-focus border token communicates controller targeting. High
contrast strengthens borders, reduced transparency resolves opaque structural surfaces, and reduced
motion removes durations without changing hierarchy.

Use `Surface`, `Button::semantic`, `RadioButton::semantic`, `SelectionIndicator`, and `Switch`
instead of restating palette values. Their ordinary, hover, pressed, keyboard-focus, controller-focus,
selected, unavailable, and disabled presentations flow through the same transient-state and paint
path as their typed actions. `Switch::with_state` represents mixed or disabled states without
pretending they can activate.

`Icon` treats supplied artwork as an alpha mask, applies a semantic tint, and requires the caller to
choose accessible labeling with `label` or exclusion with `decorative`. Application identity artwork
can continue to use `Image` without semantic recoloring. Button labels remain single-line by default;
call `max_lines` when wrapping is intentional.

## Settings composition

`ResponsiveNavigation` is the shared settings navigation contract. Give it keyed destinations and
the application-owned active destination; it derives wide sidebar and narrow reversible navigation
presentations, independent navigation/content controller scopes, headers, section labels, and
leading visuals from the same declaration. Applications retain only meaningful destination state,
not a parallel pane mode. Locale direction can be passed without reversing numerals or artwork.

`SettingsSection`, `SettingsCard`, `SettingsListCard`, `SettingsRow`, `SliderField`, `SelectField`,
`FieldGroup`, and `InlineButtonGroup` provide the shared hierarchy and form grammar. A row is not
interactive by default; call `activate` only when activating the full row is identical to its
trailing control. `SettingsStatus` exposes unavailable, validation, restart-required, and error
states both visually and to accessibility adapters.

`SettingsSearchEntry` stores localized page, section, and control labels plus a stable target.
`search_settings` returns deterministic relevance order, while `disambiguated_label` ensures that
identically named controls retain page and section context. Tabs expose their selected state and
controlled panel identity through the backend-neutral accessibility record.

## Start Menu composition

`StartMenuShell` provides a bounded wide two-pane layout and a reversible single-pane narrow layout;
the owning application chooses the active narrow pane. `SectionHeader`, `ShortcutRow`,
`ProjectStatusRow`, `AccountSummaryRow`, `SessionActionRow`, `LauncherSearchField`, and
`CompactIconTile` supply the product-neutral hierarchy used by desktop launchers without owning
application, project, account, or session policy.

Use `ReadingDirection` for semantic pane, leading/trailing, and chevron mirroring. Structural icons
belong in the row's fixed optical slot, while application artwork remains caller-owned. Every
actionable row emits the same typed message through pointer, keyboard, controller, and accessibility
activation; unavailable or disabled rows keep their textual state but expose no action.

Reusable components are ordinary typed functions. `#[component]` makes their named properties
available to `ui!`, rejects unknown, duplicate, and missing properties at the invocation, permits
property reordering, and treats `Option<T>` parameters as optional properties defaulting to
`None`.

## Layout and box model

Measurement is headless and side-effect free. A component first resolves its preferred size under
`Constraints`; placement assigns a finite nonnegative allocated rectangle; painting, hit regions,
and accessibility geometry are emitted from that placement. Content is surrounded by padding
inside the allocated box. Borders paint inside the allocated box and do not increase intrinsic
content size. Backgrounds, radii, clipping, and hit testing use the allocated box, while children
use its padding-inset content box.

`Length` distinguishes automatic, pixel, percentage, fill, fractional, minimum-content, and
maximum-content sizing. Percent and fill fall back to intrinsic size under an indefinite parent.
Rows and columns enforce basis, grow, shrink, minimums, maximums, alignment, justification, gaps,
and overflow without negative geometry. Grids support fixed, automatic, fractional, min/max,
repeated, and responsive auto-fit tracks.

`UiFrame::layout_with_diagnostics` records bounded, deduplicated structured diagnostics and a
read-only `ResolvedLayout`. `enable_diagnostic_overlay` appends a separate inspection paint phase
without altering placement or hit testing. `deterministic_snapshot` provides headless geometry
snapshots with native handles, pointers, timestamps, and cache identities omitted.

## Bounded custom painting

Ordinary application UI uses the components in the prelude. Genuinely graphical content uses
`CustomPaint`; its callback receives only the allocated rectangle, and frame resolution discards
commands outside that rectangle as well as clip-stack and overlay commands. Identity, semantics,
accessibility, pointer/controller actions, and context actions remain declarations on the component.
Raw commands are intentionally available only through the explicit `backend` module used by custom
painters and platform presenters.

~~~rust
use nickel_ui::backend::PaintCommand;
use nickel_ui::prelude::*;
use nickel_ui::Rect;

#[derive(Clone)]
enum Message { Activate }

fn paint(bounds: Rect) -> Vec<PaintCommand> {
    vec![PaintCommand::Fill { rect: bounds, color: 0x8b5cf6 }]
}

let _graph = CustomPaint::new(paint)
    .id("graph")
    .width(160.0)
    .height(80.0)
    .semantic_role(nickel_ui::SemanticRole::Button)
    .accessibility_label("Open graph")
    .message(Message::Activate);
~~~

## Source-local compile errors

Unknown properties are rejected at their invocation:

~~~compile_fail
use nickel_ui::prelude::*;
let _ = ui! { <Text unknown_property={3}>{"No"}</Text> };
~~~

Duplicate properties and missing required event properties are rejected by `ui!`:

~~~compile_fail
use nickel_ui::prelude::*;
let _ = ui! { <Column gap={2.0} gap={3.0} /> };
~~~

~~~compile_fail
use nickel_ui::prelude::*;
let _ = ui! { <Button>{"Save"}</Button> };
~~~

Components that do not own children reject them:

~~~compile_fail
use nickel_ui::prelude::*;
#[derive(Clone)] enum Message { Set(f32) }
fn set(value: f32) -> Message { Message::Set(value) }
let _ = ui! { <Slider value={0.5} on_change={set}><Text>{"No"}</Text></Slider> };
~~~

Message mapper and identity types remain ordinary checked Rust types:

~~~compile_fail
use nickel_ui::prelude::*;
#[derive(Clone)] enum Message { Set(String) }
fn set(value: String) -> Message { Message::Set(value) }
let _ = ui! { <Slider value={0.5} on_change={set} /> };
~~~

~~~compile_fail
use nickel_ui::prelude::*;
struct NotAnId;
let _ = ui! { <Text id={NotAnId}>{"No"}</Text> };
~~~

Custom-paint callbacks must accept the allocated rectangle and return only backend paint commands:

~~~compile_fail
use nickel_ui::prelude::*;
use nickel_ui::Rect;
fn invalid_paint(_: Rect) -> u32 { 7 }
let _ = CustomPaint::<()>::new(invalid_paint);
~~~

Semantic selectors are typed; renderer strings cannot stand in for semantic roles:

~~~compile_fail
use nickel_ui::{SemanticSelector};
let _ = SemanticSelector::RoleAndName {
    role: "button",
    name: "Save".into(),
};
~~~

Ordinary consumers cannot import the renderer command stream from the authoring root:

~~~compile_fail
use nickel_ui::PaintCommand;
let _ = std::mem::size_of::<PaintCommand>();
~~~

~~~compile_fail
use nickel_ui::ui::PaintCommand;
let _ = std::mem::size_of::<PaintCommand>();
~~~

Accessibility and action declarations retain their exact public types:

~~~compile_fail
use nickel_ui::prelude::*;
let _ = Container::<()>::new().semantic_role("button");
~~~

Consumer crates cannot bypass production navigation transitions by mutating focus or scope state:

~~~compile_fail
use nickel_ui::{NavigationScope, UiId, UiStateStore};
let mut state = UiStateStore::default();
state.navigation_mut().set_controller_scope(Some(UiId::from("private-scope")));
~~~

Navigation topology cannot be inferred from identifier-string prefixes:

~~~compile_fail
use nickel_ui::NavigationScope;
let _ = NavigationScope::group().parent_prefix("root/sidebar");
~~~

~~~compile_fail
use nickel_ui::backend::PaintCommand;
use nickel_ui::prelude::*;
use nickel_ui::Rect;
#[derive(Clone)] enum Message { Activate }
fn paint(bounds: Rect) -> Vec<PaintCommand> {
    vec![PaintCommand::Fill { rect: bounds, color: 0 }]
}
let _ = CustomPaint::<Message>::new(paint).message("Activate");
~~~
