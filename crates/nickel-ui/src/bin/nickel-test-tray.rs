#[cfg(target_os = "linux")]
use std::{thread, time::Duration};

#[cfg(target_os = "linux")]
struct TestTray;

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl TestTray {
    fn activate(&self, _x: i32, _y: i32) {
        println!("nickel-test-tray: activated");
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "nickel-test-tray"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        "Nickel Test Tray"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        let bytes = (0..32 * 32)
            .flat_map(|index| {
                let x = index % 32;
                let y = index / 32;
                let (red, green, blue) = if (x / 8 + y / 8) % 2 == 0 {
                    (55, 200, 255)
                } else {
                    (245, 190, 55)
                };
                [255, red, green, blue]
            })
            .collect();
        vec![(32, 32, bytes)]
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::blocking::Connection::session()?;
    connection.request_name("org.nickel.TestTray")?;
    connection
        .object_server()
        .at("/StatusNotifierItem", TestTray)?;
    let watcher = zbus::blocking::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )?;
    watcher.call_method("RegisterStatusNotifierItem", &("org.nickel.TestTray"))?;
    println!("nickel-test-tray: registered; press Ctrl-C to stop");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {}
