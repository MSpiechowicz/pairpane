//! The walk: both sides are listed one folder at a time and their names paired, so each folder is read once per side. A
//! folder that both sides have is entered (with `subfolders`); a folder only one side has is one row, with the totals of
//! what is in it and never entered row by row. A link is a row and never followed. Rows go out in batches, and every entry
//! looks at the stop flag.

use std::collections::HashMap;

use crate::model::{CompareFacts, CompareRow, CompareStatus, EntryKind};
use crate::ops::MAX_DEPTH;

use super::out::Scan;
use super::side::{Cursor, Item, Listing};
use super::status;

/// The two sides of a name: what the left one has and what the right one has.
type Pair = (Option<Item>, Option<Item>);

impl Scan<'_> {
    /// Compares the folder `left` with the folder `right`, whose path below the compared roots is `rel` (`""` for the roots),
    /// `depth` levels down.
    pub(super) fn folder(&mut self, left: &Cursor<'_>, right: &Cursor<'_>, rel: &str, depth: usize) {
        if self.halted() {
            return;
        }

        let hidden = self.options.hidden;
        let (Some(one), Some(other)) = (self.listed(left, hidden), self.listed(right, hidden)) else {
            return;
        };
        self.folders += 1;

        for pair in self.pair(one, other) {
            if self.halted() {
                return;
            }

            self.name(left, right, (rel, depth), pair);
            self.tick();
        }
    }

    /// The listing of `cursor`; `None` (counted as skipped) when it cannot be read.
    fn listed(&mut self, cursor: &Cursor<'_>, hidden: bool) -> Option<Listing> {
        match cursor.list(hidden) {
            Ok(listing) => {
                self.skipped = self.skipped.saturating_add(listing.skipped);

                Some(listing)
            }
            Err(_) => {
                self.skipped += 1;

                None
            }
        }
    }

    /// Pairs the entries of the two sides by name (as the file system compares names), ordered by name. Of two names of one
    /// side that fold together only the first is kept; the other is counted as skipped.
    fn pair(&mut self, left: Listing, right: Listing) -> Vec<Pair> {
        let mut slots: Vec<Pair> = Vec::with_capacity(left.items.len().max(right.items.len()));
        let mut places: HashMap<String, usize> = HashMap::new();

        for item in left.items {
            let key = self.key(&item.name);

            if places.contains_key(&key) {
                self.skipped += 1;
                continue;
            }
            places.insert(key, slots.len());
            slots.push((Some(item), None));
        }

        for item in right.items {
            let key = self.key(&item.name);

            match places.get(&key) {
                Some(&at) if slots[at].1.is_none() => slots[at].1 = Some(item),
                Some(_) => self.skipped += 1,
                None => {
                    places.insert(key, slots.len());
                    slots.push((None, Some(item)));
                }
            }
        }

        slots.sort_by(|a, b| first_name(a).cmp(first_name(b)));

        slots
    }

    /// A name as it is paired: folded when the file system ignores case (or the walk was asked to).
    fn key(&self, name: &str) -> String {
        if self.fold {
            name.to_lowercase()
        } else {
            name.to_owned()
        }
    }

    /// Settles one name of the folder at `rel`.
    fn name(&mut self, left: &Cursor<'_>, right: &Cursor<'_>, (rel, depth): (&str, usize), pair: Pair) {
        match pair {
            (Some(one), Some(other)) => {
                self.looked_at(&one.facts);
                self.looked_at(&other.facts);

                self.both(left, right, (join(rel, &one.name), depth), (one, other));
            }
            (Some(item), None) => {
                self.looked_at(&item.facts);

                let path = join(rel, &item.name);
                self.alone(left, item, (path, depth), CompareStatus::LeftOnly);
            }
            (None, Some(item)) => {
                self.looked_at(&item.facts);

                let path = join(rel, &item.name);
                self.alone(right, item, (path, depth), CompareStatus::RightOnly);
            }
            (None, None) => {}
        }
    }

    /// Counts a file looked at.
    fn looked_at(&mut self, facts: &CompareFacts) {
        if facts.kind == EntryKind::File {
            self.files += 1;

            // With content compared the bytes are those read, counted where they are read.
            if !self.options.content {
                self.bytes = self.bytes.saturating_add(facts.size);
            }
        }
    }

    /// A name both sides have: two folders are entered (or, without `subfolders`, are one row that says both have it); anything
    /// else is one row with its status.
    fn both(
        &mut self,
        left: &Cursor<'_>,
        right: &Cursor<'_>,
        (path, depth): (String, usize),
        (one, other): (Item, Item),
    ) {
        if one.facts.kind == EntryKind::Dir && other.facts.kind == EntryKind::Dir {
            if !self.options.subfolders {
                return self.row(path, (one.facts, other.facts), CompareStatus::Equal);
            }
            if depth + 1 >= MAX_DEPTH {
                self.skipped += 1;

                return;
            }

            return self.folder(&left.child(&one.name), &right.child(&other.name), &path, depth + 1);
        }

        let equal = self.content_equal(left, right, (&one, &other));
        // A comparison stopped in the middle of a file has no answer for it.
        if self.cancelled() {
            return;
        }

        let status = status(&one.facts, &other.facts, equal);

        self.row(path, (one.facts, other.facts), status);
    }

    /// Whether the two files hold the same bytes, when `content` is asked for, both are files of one size and both are on
    /// disk; `None` when not asked or not told.
    fn content_equal(&mut self, left: &Cursor<'_>, right: &Cursor<'_>, (one, other): (&Item, &Item)) -> Option<bool> {
        let regular = one.facts.kind == EntryKind::File && other.facts.kind == EntryKind::File;
        if !self.options.content || !regular || one.facts.size != other.facts.size {
            return None;
        }

        let (Some(one_folder), Some(other_folder)) = (left.folder(), right.folder()) else {
            return None;
        };

        self.same_content((one_folder, &one.name), (other_folder, &other.name), one.facts.size)
    }

    fn row(&mut self, path: String, (left, right): (CompareFacts, CompareFacts), status: CompareStatus) {
        self.push(CompareRow {
            path,
            left: Some(left),
            right: Some(right),
            status,
        });
    }

    /// A name only one side has (`status` says which): one row, and for a folder the totals of what is in it.
    fn alone(&mut self, cursor: &Cursor<'_>, item: Item, (path, depth): (String, usize), status: CompareStatus) {
        let mut facts = item.facts;

        if facts.kind == EntryKind::Dir {
            (facts.files, facts.bytes) = self.totals(&cursor.child(&item.name), depth + 1);
        }
        // A comparison stopped in the middle of a folder has no totals for it.
        if self.cancelled() {
            return;
        }

        let (left, right) = if status == CompareStatus::LeftOnly {
            (Some(facts), None)
        } else {
            (None, Some(facts))
        };

        self.push(CompareRow {
            path,
            left,
            right,
            status,
        });
    }

    /// The files in the folder `root` (`depth` levels down) and below it, and their bytes: all of it, hidden entries too, as a
    /// copy of the folder copies them. No link is followed, and the depth is bounded.
    fn totals(&mut self, root: &Cursor<'_>, depth: usize) -> (u64, u64) {
        let (mut files, mut bytes) = (0_u64, 0_u64);
        let mut pending = vec![(root.clone(), depth)];

        while let Some((cursor, depth)) = pending.pop() {
            if self.cancelled() {
                break;
            }
            let Some(listing) = self.listed(&cursor, true) else {
                continue;
            };
            self.folders += 1;

            for item in listing.items {
                match item.facts.kind {
                    EntryKind::File => {
                        files += 1;
                        bytes = bytes.saturating_add(item.facts.size);
                        self.files += 1;
                    }
                    EntryKind::Dir if depth + 1 < MAX_DEPTH => pending.push((cursor.child(&item.name), depth + 1)),
                    EntryKind::Dir => self.skipped += 1,
                    EntryKind::Other => {}
                }
            }
            self.tick();
        }

        (files, bytes)
    }
}

/// The name of a pair: whichever side has it.
fn first_name(pair: &Pair) -> &str {
    match pair {
        (Some(item), _) | (None, Some(item)) => &item.name,
        (None, None) => "",
    }
}

/// The path of `name` in the folder at `rel`.
fn join(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}
