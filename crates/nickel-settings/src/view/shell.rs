use super::*;

impl SettingsApp {
    pub(crate) fn handle_controller_action(&mut self, action: ControllerAction) {
        if self.navigation.handle(action) {
            self.controller_page = self.page;
            self.request_redraw();
            return;
        }
        if self.navigation.pane() == NavigationPane::Sidebar {
            self.controller_page = match action {
                ControllerAction::Up => self.controller_page.previous(),
                ControllerAction::Down => self.controller_page.next(),
                ControllerAction::Confirm => {
                    self.page = self.controller_page;
                    self.controller_page
                }
                ControllerAction::Cancel => {
                    self.running = false;
                    self.controller_page
                }
                _ => self.controller_page,
            };
            self.request_redraw();
        }
    }

    pub(crate) fn build_ui(&self, width: f32, height: f32) -> UiTree<SettingsMessage> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let (title, subtitle) = match self.page {
            SettingsPage::Display => (
                self.localizer.text("settings-display-title"),
                self.localizer.text("settings-display-subtitle"),
            ),
            SettingsPage::Bar => (
                self.localizer.text("settings-bar-title"),
                self.localizer.text("settings-bar-subtitle"),
            ),
            SettingsPage::Appearance => (
                self.localizer.text("settings-appearance-title"),
                self.localizer.text("settings-appearance-subtitle"),
            ),
            SettingsPage::Network => (
                self.localizer.text("settings-network-title"),
                self.localizer.text("settings-network-subtitle"),
            ),
            SettingsPage::Bluetooth => (
                self.localizer.text("settings-bluetooth-title"),
                self.localizer.text("settings-bluetooth-subtitle"),
            ),
        };
        let header = PageHeader::new(theme, title, subtitle);
        let selected_page = if self.navigation.pane() == NavigationPane::Sidebar {
            self.controller_page
        } else {
            self.page
        };
        let display_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Display),
            self.localizer.text("settings-nav-display"),
            selected_page == SettingsPage::Display,
        );
        let bar_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bar),
            self.localizer.text("settings-nav-bar"),
            selected_page == SettingsPage::Bar,
        );
        let appearance_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Appearance),
            self.localizer.text("settings-nav-appearance"),
            selected_page == SettingsPage::Appearance,
        );
        let network_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Network),
            self.localizer.text("settings-nav-network"),
            selected_page == SettingsPage::Network,
        );
        let bluetooth_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bluetooth),
            self.localizer.text("settings-nav-bluetooth"),
            selected_page == SettingsPage::Bluetooth,
        );
        let mut navigation = SettingsNavigation::new(theme, SIDEBAR_WIDTH as f32)
            .section(theme, self.localizer.text("settings-nav-section-system"))
            .item(display_button)
            .section(
                theme,
                self.localizer.text("settings-nav-section-personalization"),
            )
            .item(bar_button)
            .item(appearance_button)
            .section(
                theme,
                self.localizer.text("settings-nav-section-connectivity"),
            )
            .item(network_button)
            .item(bluetooth_button);
        if self.controller.connected() {
            navigation = navigation.child(ui! {
                <Container grow={1.0} padding={Insets {
                    top: 12.0, right: 0.0, bottom: 0.0, left: 8.0,
                }}>
                    <ShoulderHints color={palette.text} muted={palette.muted} />
                </Container>
            });
        }

        let content = match self.page {
            SettingsPage::Display => AnyView::new(self.display_components()),
            SettingsPage::Bar => AnyView::new(self.bar_components()),
            SettingsPage::Appearance => AnyView::new(self.appearance_components()),
            SettingsPage::Network => AnyView::new(self.network_components()),
            SettingsPage::Bluetooth => AnyView::new(self.bluetooth_components()),
        };
        let root = SettingsShell::new(theme, width, navigation, header, content);
        UiTree::layout(root, UiRect::new(0.0, 0.0, width, height))
    }
}
