# Nickel Cupertino Dock

This derived shell keeps the default Nickel launcher, Settings, Quick Settings,
notifications, and companion surfaces while replacing the edge taskbar with a
centered floating dock.

The dock uses Nickel's native CSS subset: translucent paint, a light border,
rounded app tiles, running indicators, a separated utility area, a soft native
box shadow, and hover geometry that magnifies icons through production layout
and hit testing. Nickel samples the compositor scene behind the floating taskbar
for native backdrop blur. The hovered icon and its two nearest neighbors use a
distance falloff and 150 ms retained-state transition; no JavaScript pointer or
animation loop is involved.

Validate the package from the repository root:

```sh
cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/nickel-cupertino-dock
```

For an isolated live preview on Linux, build the development tools and start the
package together with its base shell:

```sh
cargo build -p nickel --bin nickel-plugin --bin nickel-nested --bin nickel-test-input --features backend-winit
target/debug/nickel-plugin dev assets/plugins/nickel-default assets/plugins/nickel-cupertino-dock
```

Select **Nickel Cupertino Dock** as the active shell from the Plugins Settings page.
