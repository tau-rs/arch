//! SQL reading, by keywords only: tables created in `migrations/*.sql`, and the tables a query
//! string touches (spec §7). No SQL parser: a statement is read for `CREATE TABLE`, `FROM`,
//! `JOIN`, `INTO`, `UPDATE` and `FOR UPDATE SKIP LOCKED`, which is what the facts need.

use arch_facts::Access;

/// A table created by a migration, with the 1-based line of its `CREATE TABLE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// Table name, unquoted, without schema.
    pub name: String,
    /// Line of the statement.
    pub line: u32,
}

/// One table a query touches.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Touch {
    /// Table name.
    pub table: String,
    /// Read (`FROM`, `JOIN`) or write (`INSERT`, `UPDATE`, `DELETE`).
    pub access: Access2,
    /// The statement inserts into the table.
    pub inserts: bool,
    /// The statement claims rows with `FOR UPDATE SKIP LOCKED`: a dequeue.
    pub dequeues: bool,
}

/// [`Access`] with an order, so touches sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access2 {
    /// Reads.
    Read,
    /// Writes.
    Write,
}

impl From<Access2> for Access {
    fn from(a: Access2) -> Access {
        match a {
            Access2::Read => Access::Read,
            Access2::Write => Access::Write,
        }
    }
}

fn words(sql: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (i, line) in sql.lines().enumerate() {
        let line = line.split("--").next().unwrap_or("");
        for w in line.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';')) {
            if !w.is_empty() {
                out.push((w.to_string(), i as u32 + 1));
            }
        }
    }
    out
}

fn table_name(word: &str) -> Option<String> {
    let w = word.rsplit('.').next()?.trim_matches(['"', '`', '[', ']']);
    let ok = !w.is_empty()
        && w.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !w.chars().next()?.is_ascii_digit();
    ok.then(|| w.to_lowercase())
}

fn is(word: &str, kw: &str) -> bool {
    word.eq_ignore_ascii_case(kw)
}

/// Tables created by a migration file.
pub fn created_tables(sql: &str) -> Vec<Created> {
    let w = words(sql);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < w.len() {
        if is(&w[i].0, "create") && is(&w[i + 1].0, "table") {
            let mut j = i + 2;
            // IF NOT EXISTS
            while j < w.len() && ["if", "not", "exists"].iter().any(|k| is(&w[j].0, k)) {
                j += 1;
            }
            if let Some(name) = w.get(j).and_then(|x| table_name(&x.0)) {
                out.push(Created { name, line: w[i].1 });
            }
            i = j;
        }
        i += 1;
    }
    out
}

/// The tables a query string touches, sorted, one row per table (a write wins over a read).
pub fn touches(sql: &str) -> Vec<Touch> {
    let w = words(sql);
    let lower: Vec<String> = w.iter().map(|x| x.0.to_lowercase()).collect();
    let skip_locked = lower.windows(2).any(|p| p[0] == "skip" && p[1] == "locked");
    let mut out: Vec<Touch> = Vec::new();
    let mut add = |name: &str, access: Access2, inserts: bool| {
        let Some(table) = table_name(name) else {
            return;
        };
        if ["select", "set", "only", "lateral", "unnest"].contains(&table.as_str()) {
            return;
        }
        match out.iter_mut().find(|t| t.table == table) {
            Some(t) => {
                t.access = t.access.max(access);
                t.inserts |= inserts;
            }
            None => out.push(Touch {
                table,
                access,
                inserts,
                dequeues: false,
            }),
        }
    };
    for i in 0..lower.len().saturating_sub(1) {
        let next = &w[i + 1].0;
        match lower[i].as_str() {
            "from" if i > 0 && lower[i - 1] == "delete" => add(next, Access2::Write, false),
            "from" | "join" => add(next, Access2::Read, false),
            "into" => add(next, Access2::Write, true),
            "update" if i == 0 || lower[i - 1] != "for" => add(next, Access2::Write, false),
            _ => {}
        }
    }
    if skip_locked {
        for t in &mut out {
            t.dequeues = true;
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_name_their_tables_with_lines() {
        let sql = "-- CREATE TABLE ghost (\nCREATE TABLE orders (\n  id UUID\n);\ncreate table if not exists public.\"order_lines\" (x int);\nCREATE INDEX i ON orders (id);";
        let got = created_tables(sql);
        assert_eq!(
            got,
            [
                Created {
                    name: "orders".into(),
                    line: 2
                },
                Created {
                    name: "order_lines".into(),
                    line: 5
                }
            ]
        );
    }

    #[test]
    fn a_query_names_what_it_reads_and_writes() {
        let t = touches(
            "SELECT o.id FROM orders o JOIN order_lines l ON l.order_id = o.id WHERE o.id = $1",
        );
        assert_eq!(
            t.iter()
                .map(|t| (t.table.as_str(), t.access))
                .collect::<Vec<_>>(),
            [("order_lines", Access2::Read), ("orders", Access2::Read)]
        );
        let t = touches("INSERT INTO outbox (kind) VALUES ($1)");
        assert!(t[0].inserts && t[0].access == Access2::Write && !t[0].dequeues);
        let t = touches("DELETE FROM outbox WHERE id = $1");
        assert_eq!((t[0].access, t[0].inserts), (Access2::Write, false));
    }

    #[test]
    fn skip_locked_is_a_dequeue_and_for_update_is_not_a_table() {
        let t = touches(
            "UPDATE outbox SET locked_at = now() WHERE id IN (SELECT id FROM outbox WHERE locked_at IS NULL LIMIT $1 FOR UPDATE SKIP LOCKED) RETURNING id",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].table, "outbox");
        assert!(t[0].dequeues && t[0].access == Access2::Write);
    }
}
