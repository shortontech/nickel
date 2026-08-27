use super::*;

impl SettingsApp {
    fn navigation_item(
        &self,
        message: SettingsMessage,
        message_key: &'static str,
        glyph: &'static str,
        selected: bool,
        palette: ThemePalette,
    ) -> impl nickel_ui::Component<SettingsMessage> {
        let label = self.localizer.text(message_key);
        let underline_width = (label.chars().count() as f32 * 8.0).clamp(24.0, 112.0);
        ui! {
            <Container width={(SIDEBAR_WIDTH - 24) as f32} height={36.0}
                padding={Insets { top: 4.0, right: 8.0, bottom: 2.0, left: 8.0 }}
                on_press={message}>
                <Row gap={10.0}>
                    <Text width={22.0} scale={1.6}
                        color={if selected { palette.accent } else { palette.muted }}>{glyph}</Text>
                    <Column gap={2.0}>
                        <Text height={20.0} scale={2.0} bold={selected} color={palette.text}>{label}</Text>
                        <Container width={underline_width} height={2.0}
                            background={if selected { palette.accent } else { palette.panel }} />
                    </Column>
                </Row>
            </Container>
        }
    }

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
        let header = ui! {
            <Row height={72.0}>
                <Container width={SIDEBAR_WIDTH as f32} height={72.0} background={palette.panel} />
                <Container grow={1.0} height={72.0} background={palette.panel} padding={Insets {
                    top: 11.0, right: 40.0, bottom: 8.0, left: 20.0,
                }}>
                    <Column gap={4.0}>
                        <Text scale={3.0} color={palette.text}>{title}</Text>
                        <Text scale={1.0} color={palette.muted}>{subtitle}</Text>
                    </Column>
                </Container>
            </Row>
        };
        let selected_page = if self.navigation.pane() == NavigationPane::Sidebar {
            self.controller_page
        } else {
            self.page
        };
        let display_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Display),
            "settings-nav-display",
            "▣",
            selected_page == SettingsPage::Display,
            palette,
        );
        let bar_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Bar),
            "settings-nav-bar",
            "▤",
            selected_page == SettingsPage::Bar,
            palette,
        );
        let appearance_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Appearance),
            "settings-nav-appearance",
            "◐",
            selected_page == SettingsPage::Appearance,
            palette,
        );
        let network_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Network),
            "settings-nav-network",
            "⌁",
            selected_page == SettingsPage::Network,
            palette,
        );
        let bluetooth_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Bluetooth),
            "settings-nav-bluetooth",
            "ᛒ",
            selected_page == SettingsPage::Bluetooth,
            palette,
        );
        let sidebar = ui! {
            <Sidebar width={SIDEBAR_WIDTH as f32}
                background={LinearGradient::vertical(theme.colors.sidebar, theme.colors.window)}
                padding={Insets { top: 20.0, right: 12.0, bottom: 12.0, left: 12.0 }} gap={4.0}>
                {display_button}{bar_button}{appearance_button}{network_button}{bluetooth_button}
                {if self.controller.connected() {
                    ui! {
                        <Container grow={1.0} padding={Insets {
                            top: 12.0, right: 0.0, bottom: 0.0, left: 8.0,
                        }}>
                            <ShoulderHints color={palette.text} muted={palette.muted} />
                        </Container>
                    }
                } else {
                    ui! { <></> }
                }}
            </Sidebar>
        };

        let content = match self.page {
            SettingsPage::Display => AnyView::new(self.display_components()),
            SettingsPage::Bar => AnyView::new(self.bar_components()),
            SettingsPage::Appearance => AnyView::new(self.appearance_components()),
            SettingsPage::Network => AnyView::new(self.network_components()),
            SettingsPage::Bluetooth => AnyView::new(self.bluetooth_components()),
        };
        let root = ui! {
            <Column height={height} background={theme.colors.window}>
                {header}
                <Row grow={1.0}>
                    {sidebar}
                    <Container grow={1.0}>{content}</Container>
                </Row>
            </Column>
        };
        UiTree::layout(root, UiRect::new(0.0, 0.0, width, height))
    }
}
