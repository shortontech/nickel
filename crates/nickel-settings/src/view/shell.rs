use super::*;

impl SettingsApp {
    fn settings_detail(&self, page: SettingsPage, width: f32) -> AnyView<SettingsMessage> {
        match page {
            SettingsPage::Display => AnyView::new(self.display_components(if width < 720.0 {
                width
            } else {
                (width - SIDEBAR_WIDTH as f32).max(0.0)
            })),
            SettingsPage::Bar => AnyView::new(self.bar_components()),
            SettingsPage::Appearance => AnyView::new(self.appearance_components()),
            SettingsPage::Network => AnyView::new(self.network_components()),
            SettingsPage::Bluetooth | SettingsPage::BluetoothPair => {
                AnyView::new(self.bluetooth_components())
            }
            SettingsPage::DefaultApps => AnyView::new(self.default_apps_components()),
            SettingsPage::OptionalFeatures => AnyView::new(self.optional_features_components()),
            SettingsPage::Plugins => AnyView::new(self.plugins_components()),
            SettingsPage::KeyboardShortcuts => self.keyboard_shortcuts_components(),
            SettingsPage::About => self.about_components(),
        }
    }

    fn settings_view_jsx(
        &self,
        width: f32,
        height: f32,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let destinations = self.navigation_destinations();
        let active = self.active_destination.map(|_| self.page);
        let selected = destinations
            .iter()
            .find(|destination| destination.page == self.page);
        let query = self.sidebar_query.trim().to_lowercase();
        let entries = self.navigation_search_entries();
        let results = search_settings(&query, &entries);
        let data = serde_json::json!({
            "width": width, "height": height,
            "active": active.is_some(),
            "query": self.sidebar_query,
            "searchPlaceholder": self.localizer.text("settings-search-placeholder"),
            "noResults": self.localizer.text("settings-search-no-results"),
            "title": selected.map_or("Settings", |destination| destination.title.as_str()),
            "subtitle": selected.map_or("", |destination| destination.subtitle.as_str()),
            "destinations": destinations.iter().filter(|destination| destination.page != SettingsPage::BluetoothPair)
                .map(|destination| serde_json::json!({
                    "id": destination.page.to_string(),
                    "label": destination.label,
                    "section": destination.section,
                    "active": active == Some(destination.page),
                })).collect::<Vec<_>>(),
            "results": results.iter().map(|result| serde_json::json!({
                "target": result.target.as_str(),
                "label": result.disambiguated_label(),
                "available": result.available,
            })).collect::<Vec<_>>(),
        });
        let content = self.settings_detail(self.page, width);
        let mut shell = self.settings_shell.borrow_mut();
        shell
            .get_or_insert_with(crate::settings_shell::SettingsShell::new)
            .as_mut()
            .map_err(|error| error.clone())?
            .render(&data, self.ui_theme(), content)
    }

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
        height: f32,
        modality: InputModality,
    ) -> AnyView<SettingsMessage> {
        if self.settings_jsx_enabled
            && self.page != SettingsPage::BluetoothPair
            && modality != InputModality::Controller
            && let Ok(view) = self.settings_view_jsx(width, height)
        {
            let _ = self.settings_plugin_memory();
            return view;
        }
        self.settings_view_native(width, height, modality)
    }

    fn settings_view_native(
        &self,
        width: f32,
        height: f32,
        modality: InputModality,
    ) -> AnyView<SettingsMessage> {
        if self.page == SettingsPage::BluetoothPair {
            let view = self.bluetooth_pairing_view();
            let _ = self.settings_plugin_memory();
            return view;
        }
        let theme = self.ui_theme();
        let declared_destinations = self.navigation_destinations();
        let destination_header = |destination: &crate::navigation_plugin::Destination| {
            let title = destination.title.clone();
            let subtitle = destination.subtitle.clone();
            if width < 720.0 {
                AnyView::new(
                    nickel_ui::Row::new()
                        .fill_width()
                        .shrink(0.0)
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
        let palette = self.palette();
        let query = self.sidebar_query.trim().to_lowercase();
        let mut navigation = SettingsNavigation::embedded_header(theme, SIDEBAR_WIDTH as f32)
            .child(SettingsSearchField::with_leading(
                theme,
                "settings-sidebar-search",
                &self.sidebar_query,
                self.localizer.text("settings-search-placeholder"),
                sidebar_search_message,
                sidebar_icon(SidebarIconKind::Search),
            ));
        if !query.is_empty() {
            let entries = self.navigation_search_entries();
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
            } else {
                navigation = navigation.child(ui! {
                    <Container padding={Insets::all(10.0)}>
                        <Text color={palette.muted} wrap={true}>
                            {self.localizer.text("settings-search-no-results")}
                        </Text>
                    </Container>
                });
            }
        }
        // ResponsiveNavigation owns the presentation and controller-pane policy. The
        // existing sidebar is retained above while search remains app-specific; destination
        // identity and all navigation activation now flow through the shared primitive.
        let destinations = declared_destinations
            .iter()
            .map(|destination| {
                let page = destination.page;
                let detail = self.settings_detail(page, width);
                let icon = match page {
                    SettingsPage::Display => SidebarIconKind::Display,
                    SettingsPage::Bar => SidebarIconKind::Bar,
                    SettingsPage::Appearance => SidebarIconKind::Appearance,
                    SettingsPage::Network => SidebarIconKind::Network,
                    SettingsPage::Bluetooth | SettingsPage::BluetoothPair => {
                        SidebarIconKind::Bluetooth
                    }
                    SettingsPage::DefaultApps => SidebarIconKind::DefaultApps,
                    SettingsPage::OptionalFeatures | SettingsPage::Plugins => {
                        SidebarIconKind::OptionalFeatures
                    }
                    SettingsPage::KeyboardShortcuts => SidebarIconKind::Keyboard,
                    SettingsPage::About => SidebarIconKind::About,
                };
                let mut entry = ResponsiveNavigationDestination::new(
                    page,
                    destination.label.clone(),
                    SettingsMessage::Navigate(page),
                    detail,
                )
                .header(destination_header(destination))
                .leading(sidebar_icon(icon))
                .visible(query.is_empty() && page != SettingsPage::BluetoothPair);
                if !destination.section.is_empty() {
                    entry = entry.section(destination.section.clone());
                }
                entry
            })
            .collect::<Vec<_>>();
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
        let _ = self.settings_plugin_memory();
        let show_controller_legend = modality == InputModality::Controller;
        if show_controller_legend {
            let legend_height = theme.sizing.control_height + theme.spacing.control * 2.0;
            AnyView::new(
                nickel_ui::Column::new()
                    .fill_width()
                    .fill_height()
                    // ResponsiveNavigation requests all available height. Bound
                    // it to the space above the fixed legend so the legend cannot
                    // be laid out below (and clipped by) the client viewport.
                    .child(
                        nickel_ui::Container::new()
                            .fill_width()
                            .height((height - legend_height).max(0.0))
                            .child(root),
                    )
                    .child(ActionLegend::new_directional(
                        theme,
                        self.controller_family,
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
