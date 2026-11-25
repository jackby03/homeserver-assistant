//! Role-based access control.
//!
//! Permissions are checked in the HTTP layer (middleware/extractors) and again
//! in the service layer for defense in depth. Roles are coarse on purpose: the
//! appliance has few administrative users.

use serde::{Deserialize, Serialize};

use hs_core::models::Role;

/// A unit of authorization checked by handlers and services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read dashboard metrics and service health.
    DashboardView,
    /// List and inspect instances and their logs.
    InstancesRead,
    /// Create, start, stop, delete instances and manage snapshots.
    InstancesManage,
    /// List/inspect deployed applications and the catalog.
    AppsRead,
    /// Deploy and remove applications.
    AppsDeploy,
    /// Read network, DNS and firewall configuration.
    NetworkRead,
    /// Change DNS, firewall, DMZ and interfaces.
    NetworkManage,
    /// Read LLM providers, tools and workflows.
    AgentRead,
    /// Manage providers, workflows and run the assistant.
    AgentManage,
    /// Read household profiles and permissions.
    ProfilesRead,
    /// Create, edit and delete profiles and biometric enrollments.
    ProfilesManage,
    /// Manage administrative users and roles.
    UsersManage,
    /// Read the audit trail.
    AuditView,
    /// Access the wall kiosk control API.
    KioskControl,
}

impl Permission {
    /// Stable identifier used in errors and the UI permission matrix.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::DashboardView => "dashboard.view",
            Self::InstancesRead => "instances.read",
            Self::InstancesManage => "instances.manage",
            Self::AppsRead => "apps.read",
            Self::AppsDeploy => "apps.deploy",
            Self::NetworkRead => "network.read",
            Self::NetworkManage => "network.manage",
            Self::AgentRead => "agent.read",
            Self::AgentManage => "agent.manage",
            Self::ProfilesRead => "profiles.read",
            Self::ProfilesManage => "profiles.manage",
            Self::UsersManage => "users.manage",
            Self::AuditView => "audit.view",
            Self::KioskControl => "kiosk.control",
        }
    }

    /// All permissions, used by the UI matrix and tests.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::DashboardView,
            Self::InstancesRead,
            Self::InstancesManage,
            Self::AppsRead,
            Self::AppsDeploy,
            Self::NetworkRead,
            Self::NetworkManage,
            Self::AgentRead,
            Self::AgentManage,
            Self::ProfilesRead,
            Self::ProfilesManage,
            Self::UsersManage,
            Self::AuditView,
            Self::KioskControl,
        ]
    }
}

/// Permissions granted to a role.
#[must_use]
pub fn permissions_for(role: Role) -> &'static [Permission] {
    match role {
        Role::Admin => Permission::all(),
        Role::Operator => &[
            Permission::DashboardView,
            Permission::InstancesRead,
            Permission::InstancesManage,
            Permission::AppsRead,
            Permission::AppsDeploy,
            Permission::NetworkRead,
            Permission::AgentRead,
            Permission::AgentManage,
            Permission::ProfilesRead,
            Permission::ProfilesManage,
            Permission::AuditView,
            Permission::KioskControl,
        ],
        Role::Viewer => &[
            Permission::DashboardView,
            Permission::InstancesRead,
            Permission::AppsRead,
            Permission::NetworkRead,
            Permission::AgentRead,
            Permission::ProfilesRead,
        ],
    }
}

/// Whether a role grants a permission.
#[must_use]
pub fn role_can(role: Role, permission: Permission) -> bool {
    permissions_for(role).contains(&permission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_has_every_permission() {
        for permission in Permission::all() {
            assert!(role_can(Role::Admin, *permission));
        }
    }

    #[test]
    fn operator_cannot_manage_network_or_users() {
        assert!(!role_can(Role::Operator, Permission::NetworkManage));
        assert!(!role_can(Role::Operator, Permission::UsersManage));
        assert!(role_can(Role::Operator, Permission::InstancesManage));
        assert!(role_can(Role::Operator, Permission::KioskControl));
    }

    #[test]
    fn viewer_is_read_only() {
        assert!(role_can(Role::Viewer, Permission::DashboardView));
        assert!(role_can(Role::Viewer, Permission::InstancesRead));
        assert!(!role_can(Role::Viewer, Permission::InstancesManage));
        assert!(!role_can(Role::Viewer, Permission::AppsDeploy));
        assert!(!role_can(Role::Viewer, Permission::KioskControl));
    }

    #[test]
    fn permission_ids_are_unique() {
        let mut ids: Vec<_> = Permission::all().iter().map(|p| p.id()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
