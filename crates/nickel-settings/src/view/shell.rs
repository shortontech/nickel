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
        let display_label = self.localizer.text("settings-nav-display");
        let bar_label = self.localizer.text("settings-nav-bar");
        let appearance_label = self.localizer.text("settings-nav-appearance");
        let network_label = self.localizer.text("settings-nav-network");
        let bluetooth_label = self.localizer.text("settings-nav-bluetooth");
        let query = self.sidebar_query.trim().to_lowercase();
        let matches = |label: &str| query.is_empty() || label.to_lowercase().contains(&query);
        let show_display = matches(&display_label);
        let show_bar = matches(&bar_label);
        let show_appearance = matches(&appearance_label);
        let show_network = matches(&network_label);
        let show_bluetooth = matches(&bluetooth_label);
        let display_selected = selected_page == SettingsPage::Display;
        let bar_selected = selected_page == SettingsPage::Bar;
        let appearance_selected = selected_page == SettingsPage::Appearance;
        let network_selected = selected_page == SettingsPage::Network;
        let bluetooth_selected = selected_page == SettingsPage::Bluetooth;
        let display_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Display),
            display_label,
            display_selected,
            sidebar_icon(SidebarIconKind::Display),
        );
        let bar_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bar),
            bar_label,
            bar_selected,
            sidebar_icon(SidebarIconKind::Bar),
        );
        let appearance_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Appearance),
            appearance_label,
            appearance_selected,
            sidebar_icon(SidebarIconKind::Appearance),
        );
        let network_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Network),
            network_label,
            network_selected,
            sidebar_icon(SidebarIconKind::Network),
        );
        let bluetooth_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bluetooth),
            bluetooth_label,
            bluetooth_selected,
            sidebar_icon(SidebarIconKind::Bluetooth),
        );
        let mut navigation = SettingsNavigation::new(theme, SIDEBAR_WIDTH as f32).child(
            SettingsSearchField::with_leading(
                theme,
                "settings-sidebar-search",
                &self.sidebar_query,
                self.localizer.text("settings-search-placeholder"),
                sidebar_search_message,
                sidebar_icon(SidebarIconKind::Search),
            ),
        );
        if show_display {
            navigation = navigation
                .section(theme, self.localizer.text("settings-nav-section-system"))
                .item(display_button);
        }
        if show_bar || show_appearance {
            navigation = navigation.section(
                theme,
                self.localizer.text("settings-nav-section-personalization"),
            );
            if show_bar {
                navigation = navigation.item(bar_button);
            }
            if show_appearance {
                navigation = navigation.item(appearance_button);
            }
        }
        if show_network || show_bluetooth {
            navigation = navigation.section(
                theme,
                self.localizer.text("settings-nav-section-connectivity"),
            );
            if show_network {
                navigation = navigation.item(network_button);
            }
            if show_bluetooth {
                navigation = navigation.item(bluetooth_button);
            }
        }
        if !(show_display || show_bar || show_appearance || show_network || show_bluetooth) {
            navigation = navigation.child(ui! {
                <Container padding={Insets::all(10.0)}>
                    <Text color={palette.muted} wrap={true}>
                        {self.localizer.text("settings-search-no-results")}
                    </Text>
                </Container>
            });
        }
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
