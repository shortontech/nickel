# Action contributor example

This surface-free package adds an action to the `commands` slot declared by
`org.example.widget-host`. The host renders its label and invokes it through a
typed request. The callback runs in this contributor's JS instance and may
request only its own granted capabilities.

Run with `nickel-plugin dev assets/plugins/example-widget-host assets/plugins/example-action-contributor`.
