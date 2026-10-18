#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;

    let connection = zbus::blocking::Connection::session()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )?;
    let mut closed = proxy.receive_signal("NotificationClosed")?;
    let (server_name, _, _, _): (String, String, String, String) =
        proxy.call("GetServerInformation", &())?;
    if server_name != "Nickel" {
        return Err(format!("notification owner is {server_name:?}, not Nickel").into());
    }
    let hints = HashMap::<String, zbus::zvariant::OwnedValue>::new();
    let id: u32 = proxy.call(
        "Notify",
        &(
            "Nickel Test",
            0_u32,
            "",
            "Nickel notification test",
            "This notification will be replaced, then closed.",
            Vec::<String>::new(),
            &hints,
            15_000_i32,
        ),
    )?;
    if id == 0 {
        return Err("notification daemon returned the reserved zero ID".into());
    }
    std::thread::sleep(std::time::Duration::from_millis(750));
    let replacement: u32 = proxy.call(
        "Notify",
        &(
            "Nickel Test",
            id,
            "",
            "Replacement succeeded",
            "Nickel owns org.freedesktop.Notifications.",
            Vec::<String>::new(),
            &hints,
            15_000_i32,
        ),
    )?;
    if replacement != id {
        return Err(
            format!("replacement changed notification ID from {id} to {replacement}").into(),
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(1_500));
    proxy.call_method("CloseNotification", &(id))?;
    let signal = closed
        .next()
        .ok_or("notification signal stream ended before NotificationClosed")?;
    let (closed_id, reason) = signal.body().deserialize::<(u32, u32)>()?;
    if (closed_id, reason) != (id, 3) {
        return Err(format!(
            "unexpected NotificationClosed payload: id={closed_id}, reason={reason}"
        )
        .into());
    }
    println!(
        "server={server_name} notification_id={id} replacement_id={replacement} closed_reason={reason}"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Nickel's notification protocol test currently supports Linux only".into())
}
