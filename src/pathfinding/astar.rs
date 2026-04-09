#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::OnceLock;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchNode {
    position: (i32, i32),
    g_score: u32,
    f_score: u32,
}

impl Ord for SearchNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_score
            .cmp(&self.f_score)
            .then_with(|| other.g_score.cmp(&self.g_score))
            .then_with(|| other.position.cmp(&self.position))
    }
}

impl PartialOrd for SearchNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn find_path<F>(
    width: usize,
    height: usize,
    start: (i32, i32),
    goal: (i32, i32),
    mut is_walkable: F,
) -> Option<Vec<(i32, i32)>>
where
    F: FnMut(i32, i32) -> bool,
{
    if !in_bounds(width, height, start) || !in_bounds(width, height, goal) {
        return None;
    }
    if start == goal {
        return Some(vec![start]);
    }
    if !is_walkable(start.0, start.1) || !is_walkable(goal.0, goal.1) {
        return None;
    }

    let mut open_set = BinaryHeap::new();
    let mut came_from = HashMap::<(i32, i32), (i32, i32)>::new();
    let mut best_g_score = HashMap::<(i32, i32), u32>::new();

    best_g_score.insert(start, 0);
    open_set.push(SearchNode {
        position: start,
        g_score: 0,
        f_score: heuristic(start, goal),
    });

    while let Some(current) = open_set.pop() {
        if current.position == goal {
            return Some(reconstruct_path(&came_from, goal));
        }

        let known_best = best_g_score
            .get(&current.position)
            .copied()
            .unwrap_or(u32::MAX);
        if current.g_score > known_best {
            continue;
        }

        for neighbor in neighbors(width, height, current.position) {
            if !is_walkable(neighbor.0, neighbor.1) {
                continue;
            }

            let tentative_g_score = current.g_score.saturating_add(1);
            let neighbor_best = best_g_score.get(&neighbor).copied().unwrap_or(u32::MAX);
            if tentative_g_score >= neighbor_best {
                continue;
            }

            came_from.insert(neighbor, current.position);
            best_g_score.insert(neighbor, tentative_g_score);
            open_set.push(SearchNode {
                position: neighbor,
                g_score: tentative_g_score,
                f_score: tentative_g_score.saturating_add(heuristic(neighbor, goal)),
            });
        }
    }

    None
}

pub fn find_tile_path(
    tiles: &[u16],
    width: usize,
    height: usize,
    start: (i32, i32),
    goal: (i32, i32),
) -> Option<Vec<(i32, i32)>> {
    find_path(width, height, start, goal, |x, y| {
        tile_at(tiles, width, height, x, y)
            .map(is_walkable_tile)
            .unwrap_or(false)
    })
}

pub fn is_walkable_tile(tile_id: u16) -> bool {
    tile_id == 0 || portal_tile_ids().contains(&tile_id)
}

fn tile_at(tiles: &[u16], width: usize, height: usize, x: i32, y: i32) -> Option<u16> {
    if !in_bounds(width, height, (x, y)) {
        return None;
    }
    let index = y as usize * width + x as usize;
    tiles.get(index).copied()
}

fn in_bounds(width: usize, height: usize, position: (i32, i32)) -> bool {
    position.0 >= 0
        && position.1 >= 0
        && position.0 < width as i32
        && position.1 < height as i32
}

fn heuristic(from: (i32, i32), to: (i32, i32)) -> u32 {
    from.0.abs_diff(to.0) + from.1.abs_diff(to.1)
}

fn neighbors(width: usize, height: usize, position: (i32, i32)) -> [(i32, i32); 4] {
    let left = if position.0 > 0 {
        (position.0 - 1, position.1)
    } else {
        position
    };
    let right = if position.0 + 1 < width as i32 {
        (position.0 + 1, position.1)
    } else {
        position
    };
    let down = if position.1 > 0 {
        (position.0, position.1 - 1)
    } else {
        position
    };
    let up = if position.1 + 1 < height as i32 {
        (position.0, position.1 + 1)
    } else {
        position
    };
    [left, right, down, up]
}

fn reconstruct_path(
    came_from: &HashMap<(i32, i32), (i32, i32)>,
    goal: (i32, i32),
) -> Vec<(i32, i32)> {
    let mut current = goal;
    let mut path = vec![current];

    while let Some(previous) = came_from.get(&current).copied() {
        current = previous;
        path.push(current);
    }

    path.reverse();
    path
}

fn portal_tile_ids() -> &'static std::collections::HashSet<u16> {
    static PORTAL_TILE_IDS: OnceLock<std::collections::HashSet<u16>> = OnceLock::new();
    PORTAL_TILE_IDS.get_or_init(load_portal_tile_ids)
}

fn load_portal_tile_ids() -> std::collections::HashSet<u16> {
    let raw = include_str!("../../block_types.json");
    let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(raw) else {
        return std::collections::HashSet::new();
    };

    entries
        .into_iter()
        .filter_map(|(key, value)| {
            let id = key.parse::<u16>().ok()?;
            let name = value.as_str()?;
            if name.contains("Portal") {
                Some(id)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{find_path, find_tile_path, is_walkable_tile};

    #[test]
    fn finds_path_on_open_grid() {
        let path = find_path(5, 5, (0, 0), (2, 2), |_, _| true).unwrap();
        assert_eq!(path.first().copied(), Some((0, 0)));
        assert_eq!(path.last().copied(), Some((2, 2)));
        assert_eq!(path.len(), 5);
    }

    #[test]
    fn avoids_blocked_tiles() {
        let path = find_path(5, 5, (0, 0), (4, 0), |x, y| {
            !matches!((x, y), (1, 0) | (2, 0) | (3, 0))
        })
        .unwrap();
        assert!(path.contains(&(0, 1)));
        assert_eq!(path.last().copied(), Some((4, 0)));
    }

    #[test]
    fn returns_none_when_goal_is_blocked() {
        let path = find_path(3, 3, (0, 0), (2, 2), |x, y| (x, y) != (2, 2));
        assert!(path.is_none());
    }

    #[test]
    fn tile_helper_uses_air_as_walkable() {
        let tiles = vec![
            0, 0, 0,
            1, 1, 0,
            0, 0, 0,
        ];
        let path = find_tile_path(&tiles, 3, 3, (0, 0), (2, 2)).unwrap();
        assert_eq!(path.first().copied(), Some((0, 0)));
        assert_eq!(path.last().copied(), Some((2, 2)));
    }

    #[test]
    fn portal_tiles_are_whitelisted() {
        assert!(is_walkable_tile(110));
        assert!(is_walkable_tile(329));
        assert!(is_walkable_tile(2904));
    }
}
