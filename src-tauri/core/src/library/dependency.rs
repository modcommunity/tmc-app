//! What a sandbox's items need, and what they cannot live beside.
//!
//! The deployment engine already resolves **file** conflicts — two mods
//! providing one path, decided by priority and reported. This is the other
//! kind: the author's own statement that their mod needs another one, or that
//! it will not work alongside a third.
//!
//! WARNINGS, NEVER REFUSALS
//! ------------------------
//! Every answer here is advisory and the UI presents it as such. Three reasons,
//! and all of them are about the data rather than the code:
//!
//!   * **The metadata is author-written and frequently wrong.** A required edge
//!     left in place after a mod absorbed its own dependency is the normal state
//!     of every mod site.
//!   * **The app cannot see everything.** A user may have the dependency
//!     installed by hand, from another launcher, or bundled inside a modpack
//!     archive. Refusing a deploy over a file the engine can plainly see on disk
//!     would be absurd.
//!   * **A refusal has no escape hatch that is not just "ignore it".** Which
//!     means it would be ignored, which means it would train people to ignore
//!     the accurate ones too.
//!
//! So the deploy reports and proceeds, and the sandbox screen shows the same
//! thing before anybody presses it.
//!
//! DIRECTION
//! ---------
//! A requirement is directional — A needs B, B does not need A. A conflict is
//! not, and the API already returns both halves of one, so a conflict edge on
//! either item finds the pair.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::logging::now_rfc3339;

use super::db::LibraryDb;
use super::sandbox::{Sandbox, SandboxMod};

/// How one item relates to another. Mirrors the contract's
/// `DependencyRelationVals`, which mirrors Prisma's `DependencyType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Relation {
    Required,
    Optional,
    Recommended,
    Conflict,
}

impl Relation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "Required",
            Self::Optional => "Optional",
            Self::Recommended => "Recommended",
            Self::Conflict => "Conflict",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "Required" => Some(Self::Required),
            "Optional" => Some(Self::Optional),
            "Recommended" => Some(Self::Recommended),
            "Conflict" => Some(Self::Conflict),
            _ => None,
        }
    }

    /// Should the app offer to add this alongside the item?
    ///
    /// Required and Recommended, not Optional — "optional" is the author saying
    /// *this works without it*, and an app that adds those anyway has turned a
    /// three-mod install into eleven.
    pub fn is_suggested(self) -> bool {
        matches!(self, Self::Required | Self::Recommended)
    }
}

/// One cached edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    /// The item that has the dependency.
    pub kind: String,
    pub item_id: i64,

    /// The other end.
    pub rel_kind: String,
    pub rel_id: i64,

    pub relation: Relation,
    pub name: String,
    pub icon: Option<String>,
    pub note: Option<String>,
}

impl Edge {
    /// The key the sandbox's own mod list uses.
    pub fn rel_key(&self) -> String {
        SandboxMod::key_for(&self.rel_kind, self.rel_id)
    }
}

/// What a sandbox's contents say about each other.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    /// Required by something in the sandbox, and not in it.
    pub missing: Vec<Edge>,
    /// Recommended by something in the sandbox, and not in it.
    pub suggested: Vec<Edge>,
    /// Both ends of a conflict are in the sandbox and both are enabled.
    pub conflicts: Vec<Conflict>,
    /// Items whose edges have never been fetched, so nothing above covers them.
    ///
    /// Named rather than hidden: "no problems found" and "we did not look" are
    /// different answers, and a screen that shows the first when it means the
    /// second is lying.
    pub unchecked: Vec<String>,
}

impl DependencyReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.conflicts.is_empty()
    }
}

/// Two items in one sandbox that their authors say do not go together.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub a_key: String,
    pub a_name: String,
    pub b_key: String,
    pub b_name: String,
    pub note: Option<String>,
}

impl LibraryDb {
    /// Replace one item's cached edges.
    ///
    /// Wholesale, in a transaction: an edge the author DELETED has to disappear,
    /// and merging would leave it warning about a requirement that no longer
    /// exists — which is worse than not knowing, because it is confidently
    /// wrong.
    pub fn dependency_set(&self, kind: &str, item_id: i64, edges: &[Edge]) -> AppResult<()> {
        let now = now_rfc3339();

        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;

            tx.execute(
                "DELETE FROM item_dependency WHERE kind = ?1 AND item_id = ?2",
                params![kind, item_id],
            )?;

            {
                let mut stmt = tx.prepare(
                    "INSERT OR REPLACE INTO item_dependency
                        (kind, item_id, rel_kind, rel_id, relation, name, icon, note, fetched_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )?;

                for edge in edges {
                    stmt.execute(params![
                        kind,
                        item_id,
                        edge.rel_kind,
                        edge.rel_id,
                        edge.relation.as_str(),
                        edge.name,
                        edge.icon,
                        edge.note,
                        now,
                    ])?;
                }
            }

            /*
             * A row recording that this item WAS checked, even when it has no
             * edges at all. Without it "no dependencies" and "never fetched"
             * are the same empty result, and the report cannot tell the user
             * which one it is.
             */
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![format!("deps:{kind}:{item_id}"), now],
            )?;

            tx.commit()?;

            Ok(())
        })
    }

    /// One item's cached edges.
    pub fn dependency_get(&self, kind: &str, item_id: i64) -> AppResult<Vec<Edge>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT kind, item_id, rel_kind, rel_id, relation, name, icon, note
                 FROM item_dependency WHERE kind = ?1 AND item_id = ?2
                 ORDER BY relation, name",
            )?;

            let rows: rusqlite::Result<Vec<Edge>> = stmt
                .query_map(params![kind, item_id], row_to_edge)?
                .collect();

            rows
        })
    }

    /// Has this item's edge list ever been fetched?
    pub fn dependency_checked(&self, kind: &str, item_id: i64) -> AppResult<bool> {
        Ok(self.meta_get(&format!("deps:{kind}:{item_id}"))?.is_some())
    }

    /// Every edge for a set of items, in one query.
    fn dependency_for(&self, items: &[(String, i64)]) -> AppResult<Vec<Edge>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        self.with(|conn| {
            // Built from a count, bound by value. The alternative — one query
            // per item — is forty round trips through a mutex for a report the
            // user is waiting on.
            let placeholders = items
                .iter()
                .map(|_| "(kind = ? AND item_id = ?)")
                .collect::<Vec<_>>()
                .join(" OR ");

            let sql = format!(
                "SELECT kind, item_id, rel_kind, rel_id, relation, name, icon, note
                 FROM item_dependency WHERE {placeholders}"
            );

            let mut stmt = conn.prepare(&sql)?;

            let mut bound: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(items.len() * 2);

            for (kind, id) in items {
                bound.push(kind);
                bound.push(id);
            }

            let rows: rusqlite::Result<Vec<Edge>> =
                stmt.query_map(bound.as_slice(), row_to_edge)?.collect();

            rows
        })
    }

    /// Check a sandbox against what its items say about each other.
    pub fn dependency_check(&self, sandbox: &Sandbox) -> AppResult<DependencyReport> {
        let mut report = DependencyReport::default();

        /*
         * DISABLED items are excluded from every direction. A mod that is in
         * the list but switched off is not deployed, so it cannot satisfy a
         * requirement and cannot conflict with anything.
         */
        let enabled: Vec<&SandboxMod> = sandbox.enabled_mods().collect();

        if enabled.is_empty() {
            return Ok(report);
        }

        let items: Vec<(String, i64)> = enabled
            .iter()
            .map(|m| (m.kind.clone(), m.item_id))
            .collect();

        for member in &enabled {
            if !self.dependency_checked(&member.kind, member.item_id)? {
                report.unchecked.push(member.name.clone());
            }
        }

        let present: std::collections::HashSet<&str> =
            enabled.iter().map(|m| m.mod_key.as_str()).collect();

        let name_of = |key: &str| {
            enabled
                .iter()
                .find(|m| m.mod_key == key)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| key.to_string())
        };

        for edge in self.dependency_for(&items)? {
            let key = edge.rel_key();
            let holder = SandboxMod::key_for(&edge.kind, edge.item_id);

            match edge.relation {
                Relation::Conflict => {
                    if present.contains(key.as_str()) {
                        /*
                         * Both halves of a conflict come back from the API, so
                         * the same pair arrives twice. Ordering the keys makes
                         * the two identical, and the dedupe below drops one.
                         */
                        let (a, b) = if holder <= key {
                            (holder.clone(), key.clone())
                        } else {
                            (key.clone(), holder.clone())
                        };

                        if !report
                            .conflicts
                            .iter()
                            .any(|c| c.a_key == a && c.b_key == b)
                        {
                            report.conflicts.push(Conflict {
                                a_name: name_of(&a),
                                b_name: name_of(&b),
                                a_key: a,
                                b_key: b,
                                note: edge.note.clone(),
                            });
                        }
                    }
                }
                Relation::Required => {
                    if !present.contains(key.as_str()) {
                        report.missing.push(edge);
                    }
                }
                Relation::Recommended => {
                    if !present.contains(key.as_str()) {
                        report.suggested.push(edge);
                    }
                }
                // Listed on the item's own page, never surfaced as a sandbox
                // problem. That is what "optional" means.
                Relation::Optional => {}
            }
        }

        // One entry per missing item however many things require it, and a
        // stable order so the list does not shuffle between renders.
        dedupe_by_key(&mut report.missing);
        dedupe_by_key(&mut report.suggested);

        report.missing.sort_by(|a, b| a.name.cmp(&b.name));
        report.suggested.sort_by(|a, b| a.name.cmp(&b.name));
        report.unchecked.sort();
        report.unchecked.dedup();

        Ok(report)
    }
}

fn dedupe_by_key(edges: &mut Vec<Edge>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    edges.retain(|edge| seen.insert(edge.rel_key()));
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        kind: row.get(0)?,
        item_id: row.get(1)?,
        rel_kind: row.get(2)?,
        rel_id: row.get(3)?,
        // An unrecognised relation degrades to `Optional`, which is the one
        // value that causes no action at all — a downgrade must not invent a
        // requirement or a conflict.
        relation: Relation::parse(&row.get::<_, String>(4)?).unwrap_or(Relation::Optional),
        name: row.get(5)?,
        icon: row.get(6)?,
        note: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Strategy;
    use crate::library::sandbox::{Environment, NewSandbox};
    use std::collections::BTreeMap;

    fn db() -> LibraryDb {
        LibraryDb::open_memory().expect("db")
    }

    fn sandbox(db: &LibraryDb) -> i64 {
        db.sandbox_create(&NewSandbox {
            app_id: 1,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: "Test".into(),
            description: None,
            environment: Environment::Client,
            strategy: Strategy::Direct,
            game_version: None,
            loader: None,
            preset: None,
            game_dir: None,
            options: BTreeMap::new(),
            cloud_sync: false,
            auto_update: true,
        })
        .expect("create")
    }

    fn edge(kind: &str, item_id: i64, rel_kind: &str, rel_id: i64, relation: Relation) -> Edge {
        Edge {
            kind: kind.into(),
            item_id,
            rel_kind: rel_kind.into(),
            rel_id,
            relation,
            name: format!("{rel_kind} {rel_id}"),
            icon: None,
            note: None,
        }
    }

    #[test]
    fn a_required_item_that_is_absent_is_reported_missing() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.dependency_set("mod", 1, &[edge("mod", 1, "mod", 2, Relation::Required)])
            .expect("set");

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].rel_id, 2);
        assert!(!report.is_clean());
    }

    #[test]
    fn a_required_item_that_is_present_is_not_reported() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");
        db.sandbox_add_mod(id, "mod", 2, "Beta").expect("add");

        db.dependency_set("mod", 1, &[edge("mod", 1, "mod", 2, Relation::Required)])
            .expect("set");
        db.dependency_set("mod", 2, &[]).expect("set");

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert!(report.missing.is_empty());
        assert!(report.is_clean());
        assert!(report.unchecked.is_empty());
    }

    /// A mod switched off is not deployed, so it can neither satisfy a
    /// requirement nor cause a conflict.
    #[test]
    fn a_disabled_item_neither_satisfies_nor_conflicts() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");
        let beta = db.sandbox_add_mod(id, "mod", 2, "Beta").expect("add");

        db.dependency_set("mod", 1, &[edge("mod", 1, "mod", 2, Relation::Required)])
            .expect("set");
        db.dependency_set("mod", 2, &[]).expect("set");

        db.sandbox_set_mod_enabled(id, &beta, false).expect("off");

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert_eq!(report.missing.len(), 1, "a disabled mod cannot satisfy it");
    }

    #[test]
    fn a_conflict_is_reported_once_however_many_edges_describe_it() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");
        db.sandbox_add_mod(id, "mod", 2, "Beta").expect("add");

        // Both halves, as the API returns them.
        db.dependency_set("mod", 1, &[edge("mod", 1, "mod", 2, Relation::Conflict)])
            .expect("set");
        db.dependency_set("mod", 2, &[edge("mod", 2, "mod", 1, Relation::Conflict)])
            .expect("set");

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert_eq!(report.conflicts.len(), 1);
        assert!(!report.is_clean());
    }

    #[test]
    fn a_conflict_with_something_not_in_the_sandbox_is_not_a_problem() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.dependency_set("mod", 1, &[edge("mod", 1, "mod", 99, Relation::Conflict)])
            .expect("set");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert!(db.dependency_check(&found).expect("check").is_clean());
    }

    /// "Optional" is the author saying it works without it. An app that added
    /// those anyway would turn a three-mod install into eleven.
    #[test]
    fn optional_is_never_surfaced_and_recommended_is_only_a_suggestion() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.dependency_set(
            "mod",
            1,
            &[
                edge("mod", 1, "mod", 2, Relation::Optional),
                edge("mod", 1, "mod", 3, Relation::Recommended),
            ],
        )
        .expect("set");

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert!(report.missing.is_empty());
        assert_eq!(report.suggested.len(), 1);
        assert_eq!(report.suggested[0].rel_id, 3);
        // A suggestion is not a problem.
        assert!(report.is_clean());
    }

    /// "No problems found" and "we did not look" are different answers.
    #[test]
    fn an_item_whose_edges_were_never_fetched_is_named() {
        let db = db();
        let id = sandbox(&db);

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert_eq!(report.unchecked, vec!["Alpha".to_string()]);

        // An empty edge list still counts as checked.
        db.dependency_set("mod", 1, &[]).expect("set");

        let report = db.dependency_check(&found).expect("check");

        assert!(report.unchecked.is_empty());
    }

    /// An edge the author deleted has to disappear, or the app warns forever
    /// about a requirement that no longer exists.
    #[test]
    fn refetching_replaces_rather_than_merges() {
        let db = db();

        db.dependency_set(
            "mod",
            1,
            &[
                edge("mod", 1, "mod", 2, Relation::Required),
                edge("mod", 1, "mod", 3, Relation::Required),
            ],
        )
        .expect("set");

        assert_eq!(db.dependency_get("mod", 1).expect("get").len(), 2);

        db.dependency_set("mod", 1, &[edge("mod", 1, "mod", 2, Relation::Required)])
            .expect("set");

        let left = db.dependency_get("mod", 1).expect("get");

        assert_eq!(left.len(), 1);
        assert_eq!(left[0].rel_id, 2);
    }

    #[test]
    fn one_missing_item_required_by_three_things_is_listed_once() {
        let db = db();
        let id = sandbox(&db);

        for n in 1..=3 {
            db.sandbox_add_mod(id, "mod", n, &format!("Mod {n}"))
                .expect("add");

            db.dependency_set("mod", n, &[edge("mod", n, "mod", 99, Relation::Required)])
                .expect("set");
        }

        let found = db.sandbox_get(id).expect("get").expect("present");
        let report = db.dependency_check(&found).expect("check");

        assert_eq!(report.missing.len(), 1);
    }
}
