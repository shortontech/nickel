# Widget host example

This window plugin declares a replaceable `metrics` Widget slot. Its JSX chooses
where to show the bounded `nickel.data.slots.metrics` array. Contributors run
in their own plugin instances; the host receives display fields only.
The `commands` Action slot renders buttons from bounded labels and routes
clicks to the contributing plugin through a typed request.
