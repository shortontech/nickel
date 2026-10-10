use serde_json::Value;
#[test]
fn native_rejection_activates_nearest_boundary_after_rollback() {
    let mut runtime = crate::create_runtime(
            r#"
            function Leaf(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>{setCount(count+1);nickel.windows.activate('provisional')}},'leaf:'+count)}
            function Inner(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:reset},'inner:'+error.message)},h(Leaf))}
            function App(){return h(ErrorBoundary,{fallback:h(Text,null,'outer')},h(Column,null,h(Inner),h(Text,null,'sibling')))}
            "#,
            None,
        )
        .unwrap();
    let initial: Value = runtime.eval_json("__nickelRender()").unwrap();
    runtime.eval("__nickelCommitRender()").unwrap();
    runtime.begin_transaction().unwrap();
    let candidate = runtime
        .dispatch_patched("__nickelDispatchBatchPatched([[0,null]])")
        .unwrap();
    assert!(matches!(candidate, super::ScheduledPatch::Patched { .. }));
    assert_eq!(runtime.take_effects().unwrap().len(), 1);
    runtime.finish_patch_render(true).unwrap();
    runtime.finish_event(true).unwrap();
    runtime.finish_transaction(false).unwrap();

    let owner = runtime
            .eval_json::<String>("JSON.stringify(Array.from(__componentRecords).find(([,record])=>record.kind.name==='Leaf')[0])")
            .unwrap();
    let captured = runtime
        .capture_native_failure(&[owner], "native shape rejected")
        .unwrap();
    assert_eq!(captured.len(), 1);
    let fallback: Value = runtime.eval_json("__nickelRender()").unwrap();
    runtime.eval("__nickelCommitRender()").unwrap();
    assert!(fallback.to_string().contains("inner:native shape rejected"));
    assert!(fallback.to_string().contains("sibling"));
    assert!(!fallback.to_string().contains("leaf:1"));
    assert_eq!(runtime.take_effects().unwrap(), Vec::<Value>::new());
    let diagnostics = runtime.boundary_diagnostics().unwrap();
    assert_eq!(diagnostics.last().unwrap()["phase"], "native-validation");
    assert_eq!(
        diagnostics.last().unwrap()["message"],
        "native shape rejected"
    );
    // The previously admitted tree remains the native authority until the
    // fallback patch is independently accepted.
    assert!(initial.to_string().contains("leaf:0"));
}

#[test]
fn session_client_copies_account_and_captures_revision_without_private_ui_requests() {
    let data = r#"{"session":{"revision":"current","account":{"displayName":"Ada","username":"ada"},"locked":false,"support":{"lock":true,"logout":true,"suspend":true,"reboot":true,"powerOff":true,"restartShell":false}}}"#;
    let mut runtime = crate::create_runtime("", Some(data)).unwrap();
    runtime.eval("nickel.session.get().account.displayName='changed';nickel.session.lock();nickel.session.logout();nickel.session.suspend();nickel.session.reboot();nickel.session.powerOff()").unwrap();
    assert_eq!(
        runtime
            .eval_json::<Value>("JSON.stringify(nickel.session.get().account)")
            .unwrap()["displayName"],
        "Ada"
    );
    assert!(runtime.eval("nickel.session.restartShell()").is_err());
    let effects = runtime.take_effects().unwrap();
    assert_eq!(effects.len(), 5);
    assert!(
        effects
            .iter()
            .all(|effect| effect["type"] == "session.perform" && effect["revision"] == "current")
    );
    runtime.set_data(r#"{"session":{"revision":"locked","account":null,"locked":true,"support":{"lock":true,"logout":true}}}"#).unwrap();
    assert!(runtime.eval("nickel.session.logout()").is_err());
    assert!(runtime.take_effects().unwrap().is_empty());
    let mut absent = crate::create_runtime("", None).unwrap();
    assert!(absent.eval("nickel.session.lock()").is_err());
}

#[test]
fn native_wallpaper_chooser_captures_revision_without_exposing_paths() {
    let mut runtime = crate::create_runtime("", Some(r#"{"wallpaper":{"available":true,"writable":true,"generation":8,"configured":{"custom_image_configured":false,"position":"fill"},"images":[],"chooser":{"available":true,"pending":false}}}"#)).unwrap();
    runtime.eval("nickel.wallpaper.chooseImage()").unwrap();
    let effects = runtime.take_effects().unwrap();
    assert_eq!(
        effects,
        vec![
            serde_json::json!({"type":"wallpaper.chooseImage","transaction":{"generation":8,"prior":{"custom_image_configured":false,"position":"fill"}}})
        ]
    );
    for blocked in [
        r#"{"wallpaper":{"available":true,"writable":false,"generation":8,"chooser":{"available":true,"pending":false}}}"#,
        r#"{"wallpaper":{"available":true,"writable":true,"generation":8,"chooser":{"available":true,"pending":true}}}"#,
        r#"{}"#,
    ] {
        runtime.set_data(blocked).unwrap();
        assert!(runtime.eval("nickel.wallpaper.chooseImage()").is_err());
        assert!(runtime.take_effects().unwrap().is_empty());
    }
}

#[test]
fn appearance_clients_copy_preferences_and_capture_observed_transactions() {
    let mut runtime = crate::create_runtime("", Some(r#"{"appearance":{"available":true,"writable":true,"generation":7,"configured":{"theme":"system","accent_hue":null,"accent_intensity":null,"reduce_transparency":false,"animations":"normal"}},"wallpaper":{"available":true,"writable":true,"generation":8,"configured":{"custom_image_configured":false,"position":"fill"},"images":[{"id":"approved"}]}}"#)).unwrap();
    runtime.eval("let preferences = nickel.appearance.get().configured; preferences.accent_hue = 271; preferences.accent_intensity = 63; nickel.appearance.set(preferences); preferences.accent_hue = 0; nickel.wallpaper.selectImage('approved'); nickel.wallpaper.setPosition('fit'); nickel.wallpaper.resetCustomImage();").unwrap();
    let effects = runtime.take_effects().unwrap();
    assert_eq!(effects[0]["type"], "appearance.set");
    assert_eq!(effects[0]["transaction"]["generation"], 7);
    assert!(effects[0]["transaction"]["prior"]["accent_hue"].is_null());
    assert_eq!(effects[0]["transaction"]["requested"]["accent_hue"], 271);
    assert_eq!(
        effects[0]["transaction"]["requested"]["accent_intensity"],
        63
    );
    assert_eq!(effects[1]["transaction"]["change"]["image_id"], "approved");
    assert_eq!(effects[2]["transaction"]["change"]["position"], "fit");
    assert_eq!(
        effects[3]["transaction"]["change"]["kind"],
        "reset_custom_image"
    );
    assert_eq!(
        runtime
            .eval_json::<serde_json::Value>(
                "JSON.stringify(nickel.appearance.get().configured.accent_hue)"
            )
            .unwrap(),
        serde_json::Value::Null
    );
    runtime.set_data(r#"{}"#).unwrap();
    assert!(runtime.eval("nickel.appearance.set({})").is_err());
    assert!(
        runtime
            .eval("nickel.wallpaper.selectImage('approved')")
            .is_err()
    );
}

#[test]
fn native_ui_clients_emit_service_operations_and_copy_clock_snapshot() {
    let mut runtime = crate::create_runtime(
        "",
        Some(r#"{"clock":{"unixMilliseconds":1770000000000,"utcOffsetMinutes":-420}}"#),
    )
    .unwrap();
    runtime.eval("nickel.clock.get().utcOffsetMinutes = 0; nickel.projects.show(); nickel.projects.toggle(); nickel.keyboard.toggle();").unwrap();
    assert_eq!(
        runtime
            .eval_json::<i32>("JSON.stringify(nickel.clock.get().utcOffsetMinutes)")
            .unwrap(),
        -420
    );
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![
            serde_json::json!({"type":"projects.show"}),
            serde_json::json!({"type":"projects.toggle"}),
            serde_json::json!({"type":"keyboard.toggle"})
        ]
    );
}

#[test]
fn run_client_captures_revision_and_preserves_parsed_command_text() {
    let mut runtime = crate::create_runtime(
        "",
        Some(r#"{"run":{"available":true,"revision":"owner:4","status":null}}"#),
    )
    .unwrap();
    runtime
        .eval(r#"nickel.run.execute(' editor "a b" ');"#)
        .unwrap();
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![
            serde_json::json!({"type":"run.execute","command":"editor \"a b\"","revision":"owner:4"})
        ]
    );
    for source in [
        "nickel.run.execute('')",
        "nickel.run.execute('a'.repeat(4097))",
        "nickel.run.execute('abc', '')",
    ] {
        assert!(runtime.eval(source).is_err());
    }
    let mut unavailable = crate::create_runtime("", None).unwrap();
    assert!(unavailable.eval("nickel.run.execute('editor')").is_err());
    assert!(unavailable.take_effects().unwrap().is_empty());
}

#[test]
fn notification_client_uses_stable_ids_and_validates_actions() {
    let mut runtime = crate::create_runtime(
        "",
        Some(r#"{"notifications":{"notification":{"id":7,"summary":"Mail"},"history":[]}}"#),
    )
    .unwrap();
    runtime.eval("nickel.notifications.get().notification.summary = 'changed'; nickel.notifications.invoke(7, 'open'); nickel.notifications.dismiss(7);").unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>("JSON.stringify(nickel.notifications.get().notification.summary)")
            .unwrap(),
        "Mail"
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(
        effects[0],
        serde_json::json!({"type":"notifications.invoke","id":7,"key":"open"})
    );
    assert_eq!(
        effects[1],
        serde_json::json!({"type":"notifications.dismiss","id":7})
    );
    for call in [
        "nickel.notifications.dismiss(0)",
        "nickel.notifications.dismiss('7')",
        "nickel.notifications.dismiss(4294967296)",
        "nickel.notifications.invoke(7, '')",
    ] {
        assert!(runtime.eval(call).is_err());
    }
    assert!(runtime.take_effects().unwrap().is_empty());
}

#[test]
fn desktop_clients_copy_snapshots_and_emit_stable_identity_actions() {
    let mut runtime = crate::create_runtime(
        "",
        Some(r#"{"windows":[{"id":"42","title":"Editor"}],"applications":[{"id":"editor"}]}"#),
    )
    .unwrap();
    runtime.eval("nickel.windows.list()[0].title = 'changed'; nickel.windows.activate('42'); nickel.applications.launch('editor'); nickel.tray.activate('mail');").unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>("JSON.stringify(nickel.windows.list()[0].title)")
            .unwrap(),
        "Editor"
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(effects[0]["type"], "windows.focus");
    assert_eq!(effects[1]["id"], "editor");
    assert_eq!(effects[2]["type"], "tray.activate");
    assert!(runtime.eval("nickel.windows.activate(42)").is_err());
    assert!(runtime.eval("nickel.audio.setVolume(101)").is_err());
    runtime.eval("nickel.audio.setVolume(25)").unwrap();
    assert_eq!(runtime.take_effects().unwrap()[0]["value"], 25);
    assert!(
        runtime
            .eval("nickel.applications.movePin('editor', 0)")
            .is_err()
    );
    runtime
        .eval("nickel.applications.movePin('editor', -1)")
        .unwrap();
    assert_eq!(runtime.take_effects().unwrap()[0]["direction"], -1);
}

#[test]
fn preferences_clients_copy_snapshots_and_emit_only_requested_patch_fields() {
    let data = serde_json::json!({"preferences":{"available":true,"writable":true,"revision":"0123456789abcdef","configured":{"barOnAllDisplays":true,"desktopCount":4}}}).to_string();
    let mut runtime = crate::create_runtime("", Some(&data)).unwrap();
    runtime.eval("nickel.preferences.get().configured.desktopCount=9; nickel.preferences.set({desktopCount:6});").unwrap();
    assert_eq!(
        runtime
            .eval_json::<u8>("JSON.stringify(nickel.preferences.get().configured.desktopCount)")
            .unwrap(),
        4
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(effects[0]["type"], "preferences.set");
    assert_eq!(effects[0]["transaction"]["revision"], "0123456789abcdef");
    assert_eq!(
        effects[0]["transaction"]["changedFields"],
        serde_json::json!(["desktopCount"])
    );
    assert_eq!(effects[0]["transaction"]["prior"]["desktopCount"], 4);
    assert_eq!(effects[0]["transaction"]["requested"]["desktopCount"], 6);
    assert_eq!(
        effects[0]["transaction"]["requested"]["barOnAllDisplays"],
        true
    );
    assert!(
        runtime
            .eval("nickel.preferences.set({theme:'dark'})")
            .is_err()
    );
    assert!(runtime.eval("nickel.preferences.set({})").is_err());
    let mut denied = crate::create_runtime("", None).unwrap();
    assert!(
        denied
            .eval("nickel.preferences.set({desktopCount:5})")
            .is_err()
    );
}

#[test]
fn plugin_metadata_edits_capture_prior_values_and_lossless_revision() {
    let mut runtime=crate::create_runtime("",Some(r#"{"plugins":{"available":true,"writable":true,"revision":"9007199254740993","plugins":[{"id":"example","settings":[{"id":"count","value":2}]}]},"system":{"available":true,"version":"0.1.0","platform":"linux","architecture":"x86_64"}}"#)).unwrap();
    runtime.eval("nickel.plugins.setSetting('example','count',3,'9007199254740993');nickel.system.get().version='mutated';").unwrap();
    assert_eq!(
        runtime.take_effects().unwrap()[0],
        serde_json::json!({"type":"plugins.setSetting","id":"example","key":"count","revision":"9007199254740993","priorValue":2,"value":3})
    );
    assert_eq!(
        runtime
            .eval_json::<String>("JSON.stringify(nickel.system.get().version)")
            .unwrap(),
        "0.1.0"
    );
    assert!(
        runtime
            .eval("nickel.plugins.setSetting('example','unknown',3,'9007199254740993')")
            .is_err()
    );
    assert!(
        runtime
            .eval("nickel.plugins.setSetting('example','count',{},'9007199254740993')")
            .is_err()
    );
    assert!(
        runtime
            .eval("nickel.plugins.setSetting('example','count',3,'1')")
            .is_err()
    );
}

#[test]
fn shell_selection_client_checks_declared_shell_and_inventory_revision() {
    let mut runtime = crate::create_runtime("", Some(r#"{"plugins":{"available":true,"writable":true,"revision":"8","plugins":[{"id":"theme","shell":true},{"id":"tool","shell":false}]}}"#)).unwrap();
    runtime
        .eval("nickel.plugins.selectShell('theme','8')")
        .unwrap();
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![serde_json::json!({"type":"plugins.selectShell","id":"theme","revision":"8"})]
    );
    assert!(
        runtime
            .eval("nickel.plugins.selectShell('tool','8')")
            .is_err()
    );
    assert!(
        runtime
            .eval("nickel.plugins.selectShell('theme','7')")
            .is_err()
    );
    let mut denied = crate::create_runtime("", None).unwrap();
    assert!(
        denied
            .eval("nickel.plugins.selectShell('theme','8')")
            .is_err()
    );
}

#[test]
fn plugins_clients_copy_inventory_and_emit_guarded_lifecycle_requests() {
    let mut runtime = crate::create_runtime("", Some(r#"{"plugins":{"available":true,"writable":true,"revision":"9007199254740993","plugins":[{"id":"example","enabled":true,"memory":{"jsHeapBytes":null}}]}}"#)).unwrap();
    runtime.eval("nickel.plugins.list()[0].enabled=false; nickel.plugins.disable('example','9007199254740993');").unwrap();
    assert!(
        runtime
            .eval_json::<bool>("JSON.stringify(nickel.plugins.list()[0].enabled)")
            .unwrap()
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(
        effects[0],
        serde_json::json!({"type":"plugins.disable","id":"example","revision":"9007199254740993","priorEnabled":true})
    );
    assert!(
        runtime
            .eval("nickel.plugins.enable('unknown','9007199254740993')")
            .is_err()
    );
    assert!(
        runtime
            .eval("nickel.plugins.enable('example','1')")
            .is_err()
    );
    assert!(
        runtime
            .eval("nickel.plugins.enable('example',9007199254740993)")
            .is_err()
    );
    let mut denied = crate::create_runtime("", None).unwrap();
    assert!(denied.eval("nickel.plugins.enable('example','1')").is_err());
    assert!(
        !denied
            .eval_json::<bool>("JSON.stringify(nickel.plugins.get().available)")
            .unwrap()
    );
}

#[test]
fn associations_clients_copy_snapshots_and_emit_expected_revision_and_handler() {
    let mut runtime = crate::create_runtime("", Some(r#"{"associations":{"available":true,"revision":"9007199254740993","targets":[{"id":"mime:text/plain","capability":"nativeConsent","canSetDefault":true,"protected":false,"effectiveHandlerId":"old.desktop","handlers":[{"id":"new.desktop","name":"New","protected":false},{"id":"protected.desktop","protected":true}]}]}}"#)).unwrap();
    runtime.eval("nickel.associations.getHandlers('mime:text/plain').handlers[0].name = 'mutated'; nickel.associations.setDefault('mime:text/plain','new.desktop','9007199254740993'); nickel.associations.openSystemSettings();").unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>("JSON.stringify(nickel.associations.list()[0].handlers[0].name)")
            .unwrap(),
        "New"
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(
        effects[0],
        serde_json::json!({"type":"associations.setDefault","targetId":"mime:text/plain","handlerId":"new.desktop","revision":"9007199254740993","expectedHandlerId":"old.desktop"})
    );
    assert_eq!(effects[1]["type"], "associations.openSystemSettings");
    assert!(
        runtime
            .eval(
                "nickel.associations.setDefault('mime:text/plain','new.desktop',9007199254740993)"
            )
            .is_err()
    );
    assert!(
        runtime
            .eval("nickel.associations.setDefault('mime:text/plain','new.desktop','1')")
            .is_err()
    );
    assert!(runtime.eval("nickel.associations.setDefault('mime:text/plain','protected.desktop','9007199254740993')").is_err());
    let mut denied = crate::create_runtime("", None).unwrap();
    assert_eq!(
        denied
            .eval_json::<serde_json::Value>("JSON.stringify(nickel.associations.get())")
            .unwrap()["available"],
        false
    );
    assert!(
        denied
            .eval("nickel.associations.getHandlers('mime:text/plain')")
            .is_err()
    );
}

#[test]
fn connectivity_clients_copy_snapshots_and_emit_revision_bound_effects() {
    let mut runtime = crate::create_runtime("", Some(r#"{"wifi":{"available":true,"revision":"0123456789abcdef","operations":{"connect":true,"disconnect":true,"setEnabled":true},"networks":[{"id":"stable-profile","name":"SSID"}]},"bluetooth":{"available":true,"revision":"fedcba9876543210","operations":{"connect":true},"devices":[{"id":"stable-device"}]}}"#)).unwrap();
    runtime.eval("nickel.wifi.listNetworks()[0].name = 'mutated'; nickel.wifi.connect('stable-profile'); nickel.bluetooth.connect('stable-device'); nickel.wifi.disconnect('stable-profile');").unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>("JSON.stringify(nickel.wifi.listNetworks()[0].name)")
            .unwrap(),
        "SSID"
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(
        effects[0],
        serde_json::json!({"type":"wifi.connect","id":"stable-profile","revision":"0123456789abcdef"})
    );
    assert_eq!(effects[1]["id"], "stable-device");
    assert_eq!(
        effects[2],
        serde_json::json!({"type":"wifi.disconnect","id":"stable-profile","revision":"0123456789abcdef"})
    );
    assert!(runtime.eval("nickel.wifi.setEnabled('yes')").is_err());
    assert!(
        runtime
            .eval("nickel.bluetooth.disconnect('stable-device')")
            .is_err()
    );
    let mut denied = crate::create_runtime("", None).unwrap();
    assert!(
        denied
            .eval("nickel.wifi.connect('stable-profile')")
            .is_err()
    );
    assert!(
        !denied
            .eval_json::<bool>("nickel.bluetooth.get().available")
            .unwrap()
    );
}

#[test]
fn application_search_client_emits_bounded_requests_and_copies_results() {
    let mut runtime = crate::create_runtime("", Some(r#"{"applicationSearch":{"available":true,"query":"ed","results":[{"id":"editor","name":"Editor"}],"total":1}}"#)).unwrap();
    runtime.eval("nickel.applications.searchResults().results[0].name='mutated'; nickel.applications.search('ed');").unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>(
                "JSON.stringify(nickel.applications.searchResults().results[0].name)"
            )
            .unwrap(),
        "Editor"
    );
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![serde_json::json!({"type":"applications.search","query":"ed"})]
    );
    assert!(
        runtime
            .eval("nickel.applications.search('x'.repeat(513))")
            .is_err()
    );
    assert!(runtime.eval("nickel.applications.search(12)").is_err());
}

#[test]
fn optional_feature_clients_copy_observations_and_capture_native_revisions() {
    let revision = "a".repeat(64);
    let data = serde_json::json!({"features":{"available":true,"revision":revision,"operations":{"setKeyboardMode":true,"setCodexEnabled":true},"keyboard":{"mode":"automatic"}},"shortcuts":{"available":true,"editable":false,"shortcuts":[{"id":"launcher"}]}});
    let mut runtime = crate::create_runtime("", Some(&data.to_string())).unwrap();
    runtime.eval("nickel.features.get().keyboard.mode='changed'; nickel.features.setKeyboardMode('enabled'); nickel.features.setCodexEnabled(false,true);").unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>("JSON.stringify(nickel.features.get().keyboard.mode)")
            .unwrap(),
        "automatic"
    );
    let effects = runtime.take_effects().unwrap();
    assert_eq!(effects[0]["revision"], revision);
    assert_eq!(effects[1]["confirmed"], true);
    assert!(
        runtime
            .eval("nickel.features.setKeyboardMode('other')")
            .is_err()
    );
    assert!(runtime.eval("nickel.features.retryCodex()").is_err());
    assert!(
        !runtime
            .eval_json::<bool>("nickel.shortcuts.get().editable")
            .unwrap()
    );
}

#[test]
fn surface_clients_emit_owned_surface_requests_and_bound_placement() {
    let mut runtime = crate::create_runtime("", None).unwrap();
    runtime.eval("nickel.surfaces.show('settings'); nickel.surfaces.focus('settings'); nickel.surfaces.setPlacement('settings', {anchor:'bottom-right',offsetX:-16}); nickel.surfaces.hide('settings')").unwrap();
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![
            serde_json::json!({"type":"surface.show","surfaceId":"settings"}),
            serde_json::json!({"type":"surface.focus","surfaceId":"settings"}),
            serde_json::json!({"type":"surface.setPlacement","surfaceId":"settings","anchor":"bottom-right","offsetX":-16,"offsetY":0}),
            serde_json::json!({"type":"surface.hide","surfaceId":"settings"}),
        ]
    );
    assert!(runtime.eval("nickel.surfaces.show('')").is_err());
    assert!(
        runtime
            .eval("nickel.surfaces.setPlacement('settings', {anchor:'center',offsetX:8193})")
            .is_err()
    );
    assert!(runtime.take_effects().unwrap().is_empty());
}

#[test]
fn displays_application_scale_and_identify_capture_current_revisions() {
    let mut runtime = crate::create_runtime("", None).unwrap();
    assert!(
        runtime
            .eval("nickel.displays.setApplicationScale({policy:'follow'})")
            .is_err()
    );
    runtime.set_data(r#"{"displays":{"revision":"0123456789abcdef","operations":{"identify":true},"application_scale":{"available":true,"revision":"fedcba9876543210","configured":{"policy":"follow"},"supported_scales":[120,180]}}}"#).unwrap();
    runtime
        .eval("nickel.displays.getApplicationScale().configured.policy='custom'")
        .unwrap();
    assert_eq!(
        runtime
            .eval_json::<String>(
                "JSON.stringify(nickel.displays.getApplicationScale().configured.policy)"
            )
            .unwrap(),
        "follow"
    );
    for script in [
        "nickel.displays.setApplicationScale({policy:'custom',scale_120:181})",
        "nickel.displays.setApplicationScale({policy:'custom',scale_120:180.5})",
        "nickel.displays.setApplicationScale({policy:'follow'},'0123456789abcdef')",
        "nickel.displays.identify('fedcba9876543210')",
    ] {
        assert!(runtime.eval(script).is_err());
    }
    assert!(runtime.take_effects().unwrap().is_empty());
    runtime.eval("nickel.displays.setApplicationScale({policy:'custom',scale_120:180});nickel.displays.identify()").unwrap();
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![
            serde_json::json!({"type":"displays.setApplicationScale","revision":"fedcba9876543210","policy":{"policy":"custom","scale_120":180}}),
            serde_json::json!({"type":"displays.identify","revision":"0123456789abcdef"})
        ]
    );
    runtime
        .set_data(r#"{"displays":{"revision":"0123456789abcdef","operations":{"identify":false}}}"#)
        .unwrap();
    assert!(runtime.eval("nickel.displays.identify()").is_err());
}

#[test]
fn displays_facade_reads_latest_host_snapshot_and_emits_layout_effect() {
    let mut runtime = crate::create_runtime("", None).unwrap();
    assert!(
        runtime
            .eval_json::<bool>("nickel.displays.get() === undefined")
            .unwrap()
    );
    runtime
        .set_data(r#"{"displays":{"generation":1,"outputs":[{"name":"HDMI-A-1"}]}}"#)
        .unwrap();
    assert_eq!(
        runtime
            .eval_json::<Value>("JSON.stringify(nickel.displays.get())")
            .unwrap(),
        serde_json::json!({"generation": 1, "outputs": [{"name": "HDMI-A-1"}]})
    );
    runtime
        .eval("nickel.displays.get().outputs[0].name = 'mutated'")
        .unwrap();
    assert_eq!(
        runtime
            .eval_json::<Value>("JSON.stringify(nickel.displays.get())")
            .unwrap(),
        serde_json::json!({"generation": 1, "outputs": [{"name": "HDMI-A-1"}]})
    );
    runtime
            .set_data(r#"{"displays":{"generation":2,"available":true,"revision":"0123456789abcdef","outputs":[]}}"#)
            .unwrap();
    assert_eq!(
        runtime
            .eval_json::<Value>("JSON.stringify(nickel.displays.get())")
            .unwrap(),
        serde_json::json!({"generation": 2, "available": true, "revision": "0123456789abcdef", "outputs": []})
    );

    assert!(runtime.eval("nickel.displays.setLayout({primary:'HDMI-A-1',placements:[{name:'HDMI-A-1',x:0,y:0,enabled:true}]}, 'fedcba9876543210')").is_err());
    runtime.eval("let requestedLayout = {primary: 'HDMI-A-1', placements: [{name: 'HDMI-A-1', x: 0, y: 0, enabled: true, scale_120: 120, transform:'rotate90', mode: {width: 1920, height: 1080, refresh_millihz: 60000}}]}; nickel.displays.setLayout(requestedLayout); requestedLayout.placements[0].x = 100; nickel.displays.confirm(); nickel.displays.revert()")
            .unwrap();
    assert_eq!(
        runtime.take_effects().unwrap(),
        vec![
            serde_json::json!({
                "type": "displays.setLayout",
                "revision": "0123456789abcdef",
                "layout": {
                    "primary": "HDMI-A-1",
                    "placements": [{
                        "name": "HDMI-A-1", "x": 0, "y": 0, "enabled": true,
                        "scale_120": 120,
                        "transform": "rotate90",
                        "mode": {"width": 1920, "height": 1080, "refresh_millihz": 60000}
                    }]
                }
            }),
            serde_json::json!({"type": "displays.confirm"}),
            serde_json::json!({"type": "displays.revert"})
        ]
    );
}

#[test]
fn displays_facade_rejects_invalid_layout_envelopes_without_effects() {
    let mut runtime = crate::create_runtime("", None).unwrap();
    for expression in [
        "nickel.displays.setLayout(null)",
        "nickel.displays.setLayout([])",
        "nickel.displays.setLayout({primary: 1, placements: []})",
        "nickel.displays.setLayout({primary: 'A', placements: []})",
        "nickel.displays.setLayout({primary: 'A', placements: Array(33).fill({})})",
    ] {
        assert!(runtime.eval(expression).is_err(), "{expression}");
    }
    assert!(runtime.take_effects().unwrap().is_empty());
}
