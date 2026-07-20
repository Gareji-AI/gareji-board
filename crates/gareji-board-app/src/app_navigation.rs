use dioxus::prelude::*;

/// Primary information spaces shown by the desktop shell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppPage {
    Overview,
    Work,
    Projects,
    Automation,
    Activity,
}

#[derive(Clone, Copy)]
struct PageMetadata {
    label: &'static str,
    title: &'static str,
    description: &'static str,
}

impl AppPage {
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Work,
        Self::Projects,
        Self::Automation,
        Self::Activity,
    ];

    const fn metadata(self) -> PageMetadata {
        match self {
            Self::Overview => PageMetadata {
                label: "Overview",
                title: "Portfolio at a glance",
                description: "Current capacity, attention signals, and the next safe action.",
            },
            Self::Work => PageMetadata {
                label: "Work",
                title: "Plan and move the work",
                description: "Kanban, assignments, and agent profiles without infrastructure noise.",
            },
            Self::Projects => PageMetadata {
                label: "Projects",
                title: "Manage connected products",
                description: "Project health and local workspace connections. Detailed Control Graphs open separately.",
            },
            Self::Automation => PageMetadata {
                label: "Automation",
                title: "Design how work moves",
                description: "Open one focused Blueprint, Portfolio, or Control Graph workspace at a time.",
            },
            Self::Activity => PageMetadata {
                label: "Activity",
                title: "Review durable evidence",
                description: "Unlinked checkpoints, delivery state, and reconciliation history.",
            },
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.metadata().label
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        self.metadata().title
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        self.metadata().description
    }
}

/// Long-running editors and catalogs that own the document rather than interrupt it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspacePage {
    AgentProfiles,
    ProjectGraphs,
    BlueprintStudio,
    PortfolioOrchestration,
}

#[derive(Clone, Copy)]
struct WorkspaceMetadata {
    title: &'static str,
    description: &'static str,
}

impl WorkspacePage {
    const fn metadata(self) -> WorkspaceMetadata {
        match self {
            Self::AgentProfiles => WorkspaceMetadata {
                title: "Agent profiles",
                description: "Edit roles, capabilities, instructions, and Skill references.",
            },
            Self::ProjectGraphs => WorkspaceMetadata {
                title: "Control Graph manager",
                description: "Bind and edit one managed project's Control Graph at a time.",
            },
            Self::BlueprintStudio => WorkspaceMetadata {
                title: "Blueprint Studio",
                description: "Author reusable project-independent approaches and applications.",
            },
            Self::PortfolioOrchestration => WorkspaceMetadata {
                title: "Portfolio orchestration",
                description: "Author schedules and bounded routes across managed projects.",
            },
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        self.metadata().title
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        self.metadata().description
    }
}

/// Visual treatment for a card that opens another page or focused workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchCardPresentation {
    Page,
    Workspace,
}

impl LaunchCardPresentation {
    const fn class_name(self) -> &'static str {
        match self {
            Self::Page => "page-link-card",
            Self::Workspace => "workspace-card",
        }
    }

    const fn action_label(self) -> &'static str {
        match self {
            Self::Page => "Open",
            Self::Workspace => "Open workspace",
        }
    }
}

/// Move the newly selected page to the top and announce its heading to keyboard users.
pub fn focus_page_start() {
    let _ = document::eval(
        r"
        requestAnimationFrame(() => {
            const stage = document.querySelector('.page-stage');
            if (!stage) return;
            const landscapeWorkbench = window.matchMedia(
                '(min-width: 1200px) and (min-height: 650px) and (min-aspect-ratio: 8/5)'
            ).matches;
            if (landscapeWorkbench) {
                window.scrollTo({ top: 0, left: 0 });
            } else {
                stage.scrollIntoView({ block: 'start', inline: 'nearest' });
            }
            const focusTarget = stage.querySelector('.workspace-page-header, .page-intro') || stage;
            focusTarget.focus({ preventScroll: true });
        });
        null;
        ",
    );
}

#[component]
pub fn AppNavigation(active: AppPage, on_select: EventHandler<AppPage>) -> Element {
    rsx! {
        nav { class: "primary-navigation", aria_label: "Primary",
            for page in AppPage::ALL {
                button {
                    key: "nav-{page.label()}",
                    class: if page == active { "active" } else { "" },
                    aria_pressed: page == active,
                    onclick: move |_| on_select.call(page),
                    span { "{page.label()}" }
                }
            }
        }
    }
}

#[component]
pub fn PageIntro(page: AppPage) -> Element {
    rsx! {
        header { class: "page-intro", tabindex: "-1",
            p { class: "kicker", "{page.label()}" }
            h2 { "{page.title()}" }
            p { "{page.description()}" }
        }
    }
}

#[component]
pub fn WorkspacePageHeader(
    workspace: WorkspacePage,
    return_page: AppPage,
    on_back: EventHandler<()>,
) -> Element {
    rsx! {
        header { class: "workspace-page-header", tabindex: "-1",
            button {
                class: "workspace-page-back",
                onclick: move |_| on_back.call(()),
                "Back to {return_page.label()}"
            }
            div {
                p { class: "kicker", "Dedicated workspace" }
                h2 { "{workspace.title()}" }
                p { "{workspace.description()}" }
            }
        }
    }
}

#[component]
pub fn LaunchCard(
    presentation: LaunchCardPresentation,
    kicker: &'static str,
    title: &'static str,
    description: &'static str,
    meta: String,
    on_open: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        article { class: presentation.class_name(),
            if presentation == LaunchCardPresentation::Workspace {
                span { class: "workspace-card-icon", aria_hidden: "true" }
            }
            p { class: "kicker", "{kicker}" }
            h3 { "{title}" }
            p { "{description}" }
            footer {
                span { "{meta}" }
                button { onclick: move |event| on_open.call(event), "{presentation.action_label()}" }
            }
        }
    }
}

/// Accessible modal shell for short supporting tasks and focused record details.
#[component]
pub fn DetailDialog(
    title: String,
    description: String,
    kicker: String,
    class_name: String,
    on_close: EventHandler<()>,
    children: Element,
) -> Element {
    use_effect(install_modal_focus_trap);
    use_drop(restore_modal_focus);

    rsx! {
        div {
            class: "modal-layer",
            onkeydown: move |event| {
                if event.key() == Key::Escape {
                    event.prevent_default();
                    event.stop_propagation();
                    close_modal(on_close);
                }
            },
            button {
                class: "modal-backdrop",
                tabindex: "-1",
                aria_label: "Close {title}",
                onclick: move |_| close_modal(on_close),
            }
            section {
                class: "{class_name}",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{title}",
                tabindex: "-1",
                id: "detail-modal",
                header { class: "detail-modal-header",
                    div {
                        p { class: "kicker", "{kicker}" }
                        h2 { "{title}" }
                        p { "{description}" }
                    }
                    button {
                        class: "modal-close",
                        id: "detail-modal-initial-focus",
                        aria_label: "Close {title}",
                        onclick: move |_| close_modal(on_close),
                        "Close"
                    }
                }
                div { class: "detail-modal-body", {children} }
            }
        }
    }
}

fn close_modal(on_close: EventHandler<()>) {
    restore_modal_focus();
    on_close.call(());
}

fn install_modal_focus_trap() {
    let _ = document::eval(
        r#"
        (() => {
            const dialog = document.getElementById('detail-modal');
            if (!dialog) return null;

            if (window.__garejiCloseDetailModal) {
                window.__garejiCloseDetailModal(false);
            }

            const modalLayer = dialog.closest('.modal-layer');
            const backgroundElements = modalLayer?.parentElement
                ? Array.from(modalLayer.parentElement.children).filter((element) => element !== modalLayer)
                : [];
            const backgroundInertState = backgroundElements.map((element) => element.hasAttribute('inert'));
            const returnFocus = document.activeElement;
            const focusableSelector = [
                'a[href]',
                'button:not([disabled])',
                'input:not([disabled])',
                'select:not([disabled])',
                'textarea:not([disabled])',
                '[tabindex]:not([tabindex="-1"])'
            ].join(',');
            const focusableElements = () => Array.from(dialog.querySelectorAll(focusableSelector))
                .filter((element) => element.getClientRects().length > 0);
            const initialFocus = () => document.getElementById('detail-modal-initial-focus')
                || focusableElements()[0]
                || dialog;

            const handleKeydown = (event) => {
                if (event.key !== 'Tab') return;
                const focusable = focusableElements();
                if (focusable.length === 0) {
                    event.preventDefault();
                    dialog.focus();
                    return;
                }
                const first = focusable[0];
                const last = focusable[focusable.length - 1];
                if (event.shiftKey && document.activeElement === first) {
                    event.preventDefault();
                    last.focus();
                } else if (!event.shiftKey && document.activeElement === last) {
                    event.preventDefault();
                    first.focus();
                }
            };
            const handleFocusIn = (event) => {
                if (!dialog.contains(event.target)) initialFocus().focus();
            };
            const cleanup = (restore = true) => {
                dialog.removeEventListener('keydown', handleKeydown);
                document.removeEventListener('focusin', handleFocusIn);
                backgroundElements.forEach((element, index) => {
                    if (!backgroundInertState[index]) element.removeAttribute('inert');
                });
                if (restore && returnFocus?.isConnected) returnFocus.focus();
                if (window.__garejiCloseDetailModal === cleanup) {
                    delete window.__garejiCloseDetailModal;
                }
            };

            backgroundElements.forEach((element) => element.setAttribute('inert', ''));
            dialog.addEventListener('keydown', handleKeydown);
            document.addEventListener('focusin', handleFocusIn);
            window.__garejiCloseDetailModal = cleanup;
            requestAnimationFrame(() => initialFocus().focus());
            return null;
        })();
        "#,
    );
}

fn restore_modal_focus() {
    let _ = document::eval(
        r"
        if (window.__garejiCloseDetailModal) {
            window.__garejiCloseDetailModal(true);
        }
        null;
        ",
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn primary_navigation_has_stable_unique_labels() {
        let labels = AppPage::ALL.map(AppPage::label);
        assert_eq!(labels[0], "Overview");
        assert_eq!(labels.into_iter().collect::<HashSet<_>>().len(), 5);
        for page in AppPage::ALL {
            assert!(!page.title().is_empty());
            assert!(!page.description().is_empty());
        }
    }

    #[test]
    fn long_running_management_surfaces_are_dedicated_workspace_pages() {
        for workspace in [
            WorkspacePage::AgentProfiles,
            WorkspacePage::ProjectGraphs,
            WorkspacePage::BlueprintStudio,
            WorkspacePage::PortfolioOrchestration,
        ] {
            assert!(!workspace.title().is_empty());
            assert!(!workspace.description().is_empty());
        }
    }

    #[test]
    fn control_graph_copy_uses_the_canonical_domain_term() {
        assert_eq!(
            WorkspacePage::ProjectGraphs.title(),
            "Control Graph manager"
        );
        assert!(AppPage::Automation.description().contains("Control Graph"));
    }

    #[test]
    fn launch_card_presentations_define_distinct_copy_and_styles() {
        assert_eq!(LaunchCardPresentation::Page.class_name(), "page-link-card");
        assert_eq!(LaunchCardPresentation::Page.action_label(), "Open");
        assert_eq!(
            LaunchCardPresentation::Workspace.class_name(),
            "workspace-card"
        );
        assert_eq!(
            LaunchCardPresentation::Workspace.action_label(),
            "Open workspace"
        );
    }
}
