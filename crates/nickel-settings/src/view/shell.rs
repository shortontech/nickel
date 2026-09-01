use super::*;

impl SettingsApp {
    #[cfg(test)]
    pub(crate) fn build_ui(&self, width: f32, height: f32) -> UiFrame<SettingsMessage> {
        self.build_ui_internal(width, height, false)
    }

    #[cfg(test)]
    pub(crate) fn build_ui_with_diagnostics(
        &self,
        width: f32,
        height: f32,
    ) -> UiFrame<SettingsMessage> {
        self.build_ui_internal(width, height, true)
    }

    pub(crate) fn settings_view(
        &self,
        width: f32,
        _height: f32,
        modality: InputModality,
    ) -> AnyView<SettingsMessage> {
        let theme = self.ui_theme();
        let destination_header = |page| {
            let (title, subtitle) = match page {
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
            if width < 720.0 {
                AnyView::new(
                    nickel_ui::Row::new()
                        .fill_width()
                        .min_height(76.0)
                        .child(
                            Button::semantic(
                                theme,
                                SettingsMessage::ShowNavigation,
                                "‹",
                                ButtonPresentation::Quiet,
                            )
                            .id("settings-show-navigation")
                            .width(40.0)
                            .min_width(40.0),
                        )
                        .child(PageHeader::new(theme, title, subtitle)),
                )
            } else {
                AnyView::new(PageHeader::new(theme, title, subtitle))
            }
        };
        let display_label = self.localizer.text("settings-nav-display");
        let bar_label = self.localizer.text("settings-nav-bar");
        let appearance_label = self.localizer.text("settings-nav-appearance");
        let network_label = self.localizer.text("settings-nav-network");
        let bluetooth_label = self.localizer.text("settings-nav-bluetooth");
        let keyboard_label = self.localizer.text("settings-nav-keyboard");
        let about_label = self.localizer.text("settings-nav-about");
        let palette = self.palette();
        let query = self.sidebar_query.trim().to_lowercase();
        let (
            show_display,
            show_bar,
            show_appearance,
            show_network,
            show_bluetooth,
            show_keyboard,
            show_about,
        ) = (false, false, false, false, false, false, false);
        let display_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Display),
            &display_label,
            false,
        );
        let bar_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bar),
            &bar_label,
            false,
        );
        let appearance_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Appearance),
            &appearance_label,
            false,
        );
        let network_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Network),
            &network_label,
            false,
        );
        let bluetooth_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::Bluetooth),
            &bluetooth_label,
            false,
        );
        let keyboard_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::KeyboardShortcuts),
            &keyboard_label,
            false,
        );
        let about_button = NavigationItem::new(
            theme,
            SettingsMessage::Navigate(SettingsPage::About),
            &about_label,
            false,
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
        if !query.is_empty()
            && !(show_display
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
        // ResponsiveNavigation owns the presentation and controller-pane policy. The
        // existing sidebar is retained above while search remains app-specific; destination
        // identity and all navigation activation now flow through the shared primitive.
        let destinations = vec![
            ResponsiveNavigationDestination::new(
                SettingsPage::Display,
                display_label,
                SettingsMessage::Navigate(SettingsPage::Display),
                self.display_components(),
            )
            .header(destination_header(SettingsPage::Display))
            .leading(sidebar_icon(SidebarIconKind::Display))
            .section(self.localizer.text("settings-nav-section-system"))
            .visible(query.is_empty()),
            ResponsiveNavigationDestination::new(
                SettingsPage::Bar,
                bar_label,
                SettingsMessage::Navigate(SettingsPage::Bar),
                self.bar_components(),
            )
            .header(destination_header(SettingsPage::Bar))
            .leading(sidebar_icon(SidebarIconKind::Bar))
            .section(self.localizer.text("settings-nav-section-personalization"))
            .visible(query.is_empty()),
            ResponsiveNavigationDestination::new(
                SettingsPage::Appearance,
                appearance_label,
                SettingsMessage::Navigate(SettingsPage::Appearance),
                self.appearance_components(),
            )
            .header(destination_header(SettingsPage::Appearance))
            .leading(sidebar_icon(SidebarIconKind::Appearance))
            .visible(query.is_empty()),
            ResponsiveNavigationDestination::new(
                SettingsPage::Network,
                network_label,
                SettingsMessage::Navigate(SettingsPage::Network),
                self.network_components(),
            )
            .header(destination_header(SettingsPage::Network))
            .leading(sidebar_icon(SidebarIconKind::Network))
            .section(self.localizer.text("settings-nav-section-connectivity"))
            .visible(query.is_empty()),
            ResponsiveNavigationDestination::new(
                SettingsPage::Bluetooth,
                bluetooth_label,
                SettingsMessage::Navigate(SettingsPage::Bluetooth),
                self.bluetooth_components(),
            )
            .header(destination_header(SettingsPage::Bluetooth))
            .leading(sidebar_icon(SidebarIconKind::Bluetooth))
            .visible(query.is_empty()),
            ResponsiveNavigationDestination::new(
                SettingsPage::KeyboardShortcuts,
                keyboard_label,
                SettingsMessage::Navigate(SettingsPage::KeyboardShortcuts),
                self.keyboard_shortcuts_components(),
            )
            .header(destination_header(SettingsPage::KeyboardShortcuts))
            .leading(sidebar_icon(SidebarIconKind::Keyboard))
            .section(self.localizer.text("settings-nav-section-support"))
            .visible(query.is_empty()),
            ResponsiveNavigationDestination::new(
                SettingsPage::About,
                about_label,
                SettingsMessage::Navigate(SettingsPage::About),
                self.about_components(),
            )
            .header(destination_header(SettingsPage::About))
            .leading(sidebar_icon(SidebarIconKind::About))
            .visible(query.is_empty()),
        ];
        let root = ResponsiveNavigation::try_new(
            theme,
            width,
            self.active_destination.map(|_| self.page),
            destinations,
        )
        .expect("settings destinations are stable and unique")
        .breakpoint(720.0)
        .direction(if self.localizer.is_right_to_left() {
            ReadingDirection::RightToLeft
        } else {
            ReadingDirection::LeftToRight
        })
        .navigation_header(navigation)
        .navigation_width(SIDEBAR_WIDTH as f32)
        .id("settings-navigation");
        let show_controller_legend = modality == InputModality::Controller;
        if show_controller_legend {
            AnyView::new(
                nickel_ui::Column::new()
                    .fill_width()
                    .fill_height()
                    .child(root)
                    .child(ActionLegend::new_directional(
                        theme,
                        Default::default(),
                        [
                            ActionLegendEntry::available(
                                SemanticControllerAction::Confirm,
                                "Select",
                            ),
                            ActionLegendEntry::available(
                                SemanticControllerAction::PreviousSection,
                                "Navigation",
                            ),
                            ActionLegendEntry::available(
                                SemanticControllerAction::NextSection,
                                "Content",
                            ),
                            ActionLegendEntry::available(SemanticControllerAction::Cancel, "Back"),
                        ],
                        if self.localizer.is_right_to_left() {
                            ReadingDirection::RightToLeft
                        } else {
                            ReadingDirection::LeftToRight
                        },
                    )),
            )
        } else {
            AnyView::new(root)
        }
    }

    #[cfg(test)]
    fn build_ui_internal(
        &self,
        width: f32,
        height: f32,
        diagnostics: bool,
    ) -> UiFrame<SettingsMessage> {
        let root = self.settings_view(width, height, InputModality::Pointer);
        let bounds = UiRect::new(0.0, 0.0, width, height);
        if diagnostics {
            UiFrame::layout_with_diagnostics(root, bounds)
        } else {
            UiFrame::layout(root, bounds)
        }
    }
}
