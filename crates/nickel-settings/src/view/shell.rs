use super::*;

impl SettingsApp {
    pub(crate) fn handle_controller_action(&mut self, action: ControllerAction) {
        if self.navigation.handle(action) {
            self.controller_page = self.page;
            self.narrow_navigation = self.navigation.pane() == NavigationPane::Sidebar;
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
        } else {
            let event = match action {
                ControllerAction::Up | ControllerAction::Left => Some(UiEvent::ControllerPrevious),
                ControllerAction::Down | ControllerAction::Right => Some(UiEvent::ControllerNext),
                ControllerAction::Confirm => Some(UiEvent::ControllerActivate),
                ControllerAction::Cancel => {
                    self.navigation.handle(ControllerAction::PreviousPane);
                    self.narrow_navigation = true;
                    None
                }
                ControllerAction::PreviousPane | ControllerAction::NextPane => None,
            };
            if let Some(event) = event {
                self.dispatch_ui_event(event);
            } else {
                self.request_redraw();
            }
        }
    }

    pub(crate) fn build_ui(&self, width: f32, height: f32) -> UiTree<SettingsMessage> {
        self.build_ui_internal(width, height, false)
    }

    #[cfg(test)]
    pub(crate) fn build_ui_with_diagnostics(
        &self,
        width: f32,
        height: f32,
    ) -> UiTree<SettingsMessage> {
        self.build_ui_internal(width, height, true)
    }

    fn build_ui_internal(
        &self,
        width: f32,
        height: f32,
        diagnostics: bool,
    ) -> UiTree<SettingsMessage> {
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
            SettingsPage::KeyboardShortcuts => (
                self.localizer.text("settings-keyboard-title"),
                self.localizer.text("settings-keyboard-subtitle"),
            ),
            SettingsPage::About => (
                self.localizer.text("settings-about-title"),
                self.localizer.text("settings-about-subtitle"),
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
        let keyboard_label = self.localizer.text("settings-nav-keyboard");
        let about_label = self.localizer.text("settings-nav-about");
        let query = self.sidebar_query.trim().to_lowercase();
        let show_display = query.is_empty();
        let show_bar = query.is_empty();
        let show_appearance = query.is_empty();
        let show_network = query.is_empty();
        let show_bluetooth = query.is_empty();
        let show_keyboard = query.is_empty();
        let show_about = query.is_empty();
        let display_selected = selected_page == SettingsPage::Display;
        let bar_selected = selected_page == SettingsPage::Bar;
        let appearance_selected = selected_page == SettingsPage::Appearance;
        let network_selected = selected_page == SettingsPage::Network;
        let bluetooth_selected = selected_page == SettingsPage::Bluetooth;
        let keyboard_selected = selected_page == SettingsPage::KeyboardShortcuts;
        let about_selected = selected_page == SettingsPage::About;
        let display_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Display),
            display_label,
            display_selected,
            sidebar_icon(SidebarIconKind::Display),
        )
        .id("nav-display");
        let bar_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bar),
            bar_label,
            bar_selected,
            sidebar_icon(SidebarIconKind::Bar),
        )
        .id("nav-bar");
        let appearance_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Appearance),
            &appearance_label,
            appearance_selected,
            sidebar_icon(SidebarIconKind::Appearance),
        )
        .id("nav-appearance");
        let network_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Network),
            network_label,
            network_selected,
            sidebar_icon(SidebarIconKind::Network),
        )
        .id("nav-network");
        let bluetooth_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bluetooth),
            bluetooth_label,
            bluetooth_selected,
            sidebar_icon(SidebarIconKind::Bluetooth),
        )
        .id("nav-bluetooth");
        let keyboard_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::KeyboardShortcuts),
            keyboard_label,
            keyboard_selected,
            sidebar_icon(SidebarIconKind::Keyboard),
        )
        .id("nav-keyboard");
        let about_button = NavigationItem::with_leading(
            theme,
            SettingsMessage::Navigate(SettingsPage::About),
            about_label,
            about_selected,
            sidebar_icon(SidebarIconKind::About),
        )
        .id("nav-about");
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
        if !query.is_empty() {
            let appearance_section = self.localizer.text("settings-interface-settings");
            let entries = [
                SettingsSearchEntry::new(
                    &appearance_label,
                    self.localizer.text("settings-appearance-mode"),
                    self.localizer.text("settings-appearance-automatic"),
                    "appearance-mode-system",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-mode-system".into(),
                    ),
                ),
                SettingsSearchEntry::new(
                    &appearance_label,
                    &appearance_section,
                    self.localizer.text("settings-appearance-starting-hue"),
                    "appearance-hue",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-hue".into(),
                    ),
                ),
                SettingsSearchEntry::new(
                    &appearance_label,
                    &appearance_section,
                    self.localizer.text("settings-appearance-color-intensity"),
                    "appearance-intensity",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-intensity".into(),
                    ),
                ),
                SettingsSearchEntry::new(
                    &appearance_label,
                    &appearance_section,
                    self.localizer.text("settings-reduce-transparency"),
                    "appearance-transparency",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-transparency".into(),
                    ),
                ),
                SettingsSearchEntry::new(
                    &appearance_label,
                    &appearance_section,
                    self.localizer.text("settings-animations"),
                    "appearance-animations",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-animations".into(),
                    ),
                ),
                SettingsSearchEntry::new(
                    &appearance_label,
                    self.localizer.text("settings-appearance-mode"),
                    self.localizer.text("settings-tab-theme"),
                    "appearance-tab-theme",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-tab-theme".into(),
                    ),
                )
                .unavailable(),
                SettingsSearchEntry::new(
                    &appearance_label,
                    self.localizer.text("settings-appearance-mode"),
                    self.localizer.text("settings-tab-fonts"),
                    "appearance-tab-fonts",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-tab-fonts".into(),
                    ),
                )
                .unavailable(),
                SettingsSearchEntry::new(
                    &appearance_label,
                    self.localizer.text("settings-appearance-mode"),
                    self.localizer.text("settings-tab-icons"),
                    "appearance-tab-icons",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-tab-icons".into(),
                    ),
                )
                .unavailable(),
                SettingsSearchEntry::new(
                    &appearance_label,
                    self.localizer.text("settings-appearance-mode"),
                    self.localizer.text("settings-tab-cursors"),
                    "appearance-tab-cursors",
                    SettingsMessage::NavigateTarget(
                        SettingsPage::Appearance,
                        "appearance-tab-cursors".into(),
                    ),
                )
                .unavailable(),
            ];
            let results = search_settings(&query, &entries);
            if !results.is_empty() {
                navigation =
                    navigation.section(theme, self.localizer.text("settings-search-results"));
                for result in results {
                    if result.available {
                        navigation = navigation.item(
                            NavigationItem::new(
                                theme,
                                result.message.clone(),
                                result.disambiguated_label(),
                                false,
                            )
                            .id(format!("search-result-{}", result.target.as_str())),
                        );
                    } else {
                        navigation = navigation.child(
                            NavigationItem::unavailable(
                                theme,
                                result.disambiguated_label(),
                                self.localizer.text("settings-search-unavailable"),
                            )
                            .id(format!("search-result-{}", result.target.as_str())),
                        );
                    }
                }
            }
        }
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
        if show_keyboard || show_about {
            navigation =
                navigation.section(theme, self.localizer.text("settings-nav-section-support"));
            if show_keyboard {
                navigation = navigation.item(keyboard_button);
            }
            if show_about {
                navigation = navigation.item(about_button);
            }
        }
        if !(show_display
            || show_bar
            || show_appearance
            || show_network
            || show_bluetooth
            || show_keyboard
            || show_about)
        {
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
            SettingsPage::KeyboardShortcuts => AnyView::new(self.keyboard_shortcuts_components()),
            SettingsPage::About => AnyView::new(self.about_components()),
        };
        let navigation_toggle = Button::semantic(
            theme,
            SettingsMessage::ToggleNavigation,
            self.localizer.text("settings-show-navigation"),
            ButtonPresentation::Quiet,
        );
        let root = SettingsShell::responsive(
            theme,
            width,
            if self.narrow_navigation {
                SettingsNarrowPane::Navigation
            } else {
                SettingsNarrowPane::Content
            },
            navigation_toggle,
            navigation,
            header,
            content,
            if self.localizer.is_right_to_left() {
                ReadingDirection::RightToLeft
            } else {
                ReadingDirection::LeftToRight
            },
        );
        let bounds = UiRect::new(0.0, 0.0, width, height);
        if diagnostics {
            UiTree::layout_with_diagnostics(root, bounds)
        } else {
            UiTree::layout(root, bounds)
        }
    }
}
