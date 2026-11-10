//! Exact port of `EndCityPieces.startHouseTower` to determine whether an End City generates an
//! End Ship (Elytra).
//!
//! `EndCityStructure` draws one initial `rot = r.next_int_bounded(4)` before calling
//! `EndCityPieces.startHouseTower(manager, pos, rot, pieces, r)`. All piece offsets and bounding
//! boxes translate identically with `pos`, so simulating from `(0, 0, 0)` reproduces the exact
//! piece tree, RNG stream and bounding-box collision checks.

use crate::random::Legacy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Template {
    BaseFloor,
    BaseRoof,
    BridgeEnd,
    BridgeGentleStairs,
    BridgePiece,
    BridgeSteepStairs,
    FatTowerBase,
    FatTowerMiddle,
    FatTowerTop,
    SecondFloor1,
    SecondFloor2,
    SecondRoof,
    Ship,
    ThirdFloor1,
    ThirdFloor2,
    ThirdRoof,
    TowerBase,
    TowerPiece,
    TowerTop,
}

impl Template {
    const fn size(self) -> (i32, i32, i32) {
        match self {
            Template::BaseFloor => (10, 4, 10),
            Template::BaseRoof => (12, 2, 12),
            Template::BridgeEnd => (5, 6, 2),
            Template::BridgeGentleStairs => (5, 7, 8),
            Template::BridgePiece => (5, 6, 4),
            Template::BridgeSteepStairs => (5, 7, 4),
            Template::FatTowerBase => (13, 4, 13),
            Template::FatTowerMiddle => (13, 8, 13),
            Template::FatTowerTop => (17, 6, 17),
            Template::SecondFloor1 | Template::SecondFloor2 => (12, 8, 12),
            Template::SecondRoof => (14, 2, 14),
            Template::Ship => (13, 24, 29),
            Template::ThirdFloor1 | Template::ThirdFloor2 => (14, 8, 14),
            Template::ThirdRoof => (16, 2, 16),
            Template::TowerBase => (7, 7, 7),
            Template::TowerPiece => (7, 4, 7),
            Template::TowerTop => (9, 5, 9),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BBox {
    min_x: i32,
    min_y: i32,
    min_z: i32,
    max_x: i32,
    max_y: i32,
    max_z: i32,
}

impl BBox {
    #[inline]
    fn intersects(&self, o: &BBox) -> bool {
        self.max_x >= o.min_x
            && self.min_x <= o.max_x
            && self.max_z >= o.min_z
            && self.min_z <= o.max_z
            && self.max_y >= o.min_y
            && self.min_y <= o.max_y
    }
}

#[inline]
fn rotate(rot: u8, p: (i32, i32, i32)) -> (i32, i32, i32) {
    match rot & 3 {
        1 => (-p.2, p.1, p.0),
        2 => (-p.0, p.1, -p.2),
        3 => (p.2, p.1, -p.0),
        _ => p,
    }
}

#[derive(Clone, Copy, Debug)]
struct Piece {
    tpl: Template,
    pos: (i32, i32, i32),
    rot: u8,
    gen_depth: i32,
    bb: BBox,
}

impl Piece {
    fn new(tpl: Template, pos: (i32, i32, i32), rot: u8) -> Piece {
        let rot = rot & 3;
        let (sx, sy, sz) = tpl.size();
        let c = rotate(rot, (sx - 1, sy - 1, sz - 1));
        let bb = BBox {
            min_x: pos.0 + c.0.min(0),
            min_y: pos.1 + c.1.min(0),
            min_z: pos.2 + c.2.min(0),
            max_x: pos.0 + c.0.max(0),
            max_y: pos.1 + c.1.max(0),
            max_z: pos.2 + c.2.max(0),
        };
        Piece { tpl, pos, rot, gen_depth: 0, bb }
    }
}

fn add_piece(prev: &Piece, offset: (i32, i32, i32), tpl: Template, rot: u8) -> Piece {
    let d = rotate(prev.rot, offset);
    Piece::new(tpl, (prev.pos.0 + d.0, prev.pos.1 + d.1, prev.pos.2 + d.2), rot)
}

fn push(list: &mut Vec<Piece>, p: Piece) -> Piece {
    list.push(p);
    p
}

#[derive(Clone, Copy)]
enum Section {
    HouseTower,
    Tower,
    TowerBridge,
    FatTower,
}

const TOWER_BRIDGES: [(u8, (i32, i32, i32)); 4] = [
    (0, (1, -1, 0)),
    (1, (6, -1, 1)),
    (3, (0, -1, 5)),
    (2, (5, -1, 6)),
];

const FAT_TOWER_BRIDGES: [(u8, (i32, i32, i32)); 4] = [
    (0, (4, -1, 0)),
    (1, (12, -1, 4)),
    (3, (0, -1, 8)),
    (2, (8, -1, 12)),
];

/// True when `EndCityPieces.startHouseTower` places a surviving `ship` piece.
pub fn has_ship(rot: u8, r: &mut Legacy) -> bool {
    let mut pieces = Vec::with_capacity(64);
    let mut ship_created = false;
    let mut p = push(&mut pieces, Piece::new(Template::BaseFloor, (0, 0, 0), rot));
    p = push(&mut pieces, add_piece(&p, (-1, 0, -1), Template::SecondFloor1, rot));
    p = push(&mut pieces, add_piece(&p, (-1, 4, -1), Template::ThirdFloor1, rot));
    p = push(&mut pieces, add_piece(&p, (-1, 8, -1), Template::ThirdRoof, rot));
    recursive_children(Section::Tower, 1, &p, (0, 0, 0), &mut pieces, r, &mut ship_created);
    pieces.iter().any(|piece| piece.tpl == Template::Ship)
}

fn recursive_children(
    sec: Section,
    depth: i32,
    parent: &Piece,
    offset: (i32, i32, i32),
    pieces: &mut Vec<Piece>,
    r: &mut Legacy,
    ship_created: &mut bool,
) -> bool {
    if depth > 8 {
        return false;
    }
    let mut local = Vec::new();
    if !generate(sec, depth, parent, offset, &mut local, r, ship_created) {
        return false;
    }
    let gen_depth = r.next_int();
    let mut collided = false;
    for child in &mut local {
        child.gen_depth = gen_depth;
        if let Some(hit) = pieces.iter().find(|p| p.bb.intersects(&child.bb))
            && hit.gen_depth != parent.gen_depth
        {
            collided = true;
            break;
        }
    }
    if !collided {
        pieces.extend(local);
        return true;
    }
    false
}

fn generate(
    sec: Section,
    depth: i32,
    parent: &Piece,
    offset: (i32, i32, i32),
    local: &mut Vec<Piece>,
    r: &mut Legacy,
    ship_created: &mut bool,
) -> bool {
    let rot = parent.rot;
    match sec {
        Section::HouseTower => {
            if depth > 8 {
                return false;
            }
            let mut p = push(local, add_piece(parent, offset, Template::BaseFloor, rot));
            let kind = r.next_int_bounded(3);
            if kind == 0 {
                push(local, add_piece(&p, (-1, 4, -1), Template::BaseRoof, rot));
            } else if kind == 1 {
                p = push(local, add_piece(&p, (-1, 0, -1), Template::SecondFloor2, rot));
                p = push(local, add_piece(&p, (-1, 8, -1), Template::SecondRoof, rot));
                recursive_children(Section::Tower, depth + 1, &p, (0, 0, 0), local, r, ship_created);
            } else if kind == 2 {
                p = push(local, add_piece(&p, (-1, 0, -1), Template::SecondFloor2, rot));
                p = push(local, add_piece(&p, (-1, 4, -1), Template::ThirdFloor2, rot));
                p = push(local, add_piece(&p, (-1, 8, -1), Template::ThirdRoof, rot));
                recursive_children(Section::Tower, depth + 1, &p, (0, 0, 0), local, r, ship_created);
            }
            true
        }
        Section::Tower => {
            let ox = 3 + r.next_int_bounded(2);
            let oz = 3 + r.next_int_bounded(2);
            let mut p = push(local, add_piece(parent, (ox, -3, oz), Template::TowerBase, rot));
            p = push(local, add_piece(&p, (0, 7, 0), Template::TowerPiece, rot));
            let mut bridge = if r.next_int_bounded(3) == 0 { Some(p) } else { None };
            let floors = 1 + r.next_int_bounded(3);
            for i in 0..floors {
                p = push(local, add_piece(&p, (0, 4, 0), Template::TowerPiece, rot));
                if i < floors - 1 && r.next_boolean() {
                    bridge = Some(p);
                }
            }
            if let Some(bp) = bridge {
                for &(brot, boff) in &TOWER_BRIDGES {
                    if r.next_boolean() {
                        let be = push(local, add_piece(&bp, boff, Template::BridgeEnd, (rot + brot) & 3));
                        recursive_children(Section::TowerBridge, depth + 1, &be, (0, 0, 0), local, r, ship_created);
                    }
                }
                push(local, add_piece(&p, (-1, 4, -1), Template::TowerTop, rot));
                true
            } else if depth == 7 {
                push(local, add_piece(&p, (-1, 4, -1), Template::TowerTop, rot));
                true
            } else {
                recursive_children(Section::FatTower, depth + 1, &p, (0, 0, 0), local, r, ship_created)
            }
        }
        Section::TowerBridge => {
            let len = r.next_int_bounded(4) + 1;
            let mut p = add_piece(parent, (0, 0, -4), Template::BridgePiece, rot);
            p.gen_depth = -1;
            local.push(p);
            let mut y = 0;
            for _ in 0..len {
                if r.next_boolean() {
                    p = push(local, add_piece(&p, (0, y, -4), Template::BridgePiece, rot));
                    y = 0;
                } else {
                    if r.next_boolean() {
                        p = push(local, add_piece(&p, (0, y, -4), Template::BridgeSteepStairs, rot));
                    } else {
                        p = push(local, add_piece(&p, (0, y, -8), Template::BridgeGentleStairs, rot));
                    }
                    y = 4;
                }
            }
            if !*ship_created && r.next_int_bounded(10 - depth) == 0 {
                let ox = -8 + r.next_int_bounded(8);
                let oz = -70 + r.next_int_bounded(10);
                push(local, add_piece(&p, (ox, y, oz), Template::Ship, rot));
                *ship_created = true;
            } else if !recursive_children(Section::HouseTower, depth + 1, &p, (-3, y + 1, -11), local, r, ship_created) {
                return false;
            }
            let mut end = add_piece(&p, (4, y, 0), Template::BridgeEnd, (rot + 2) & 3);
            end.gen_depth = -1;
            local.push(end);
            true
        }
        Section::FatTower => {
            let mut p = push(local, add_piece(parent, (-3, 4, -3), Template::FatTowerBase, rot));
            p = push(local, add_piece(&p, (0, 4, 0), Template::FatTowerMiddle, rot));
            for _ in 0..2 {
                if r.next_int_bounded(3) == 0 {
                    break;
                }
                p = push(local, add_piece(&p, (0, 8, 0), Template::FatTowerMiddle, rot));
                for &(brot, boff) in &FAT_TOWER_BRIDGES {
                    if r.next_boolean() {
                        let be = push(local, add_piece(&p, boff, Template::BridgeEnd, (rot + brot) & 3));
                        recursive_children(Section::TowerBridge, depth + 1, &be, (0, 0, 0), local, r, ship_created);
                    }
                }
            }
            push(local, add_piece(&p, (-2, 8, -2), Template::FatTowerTop, rot));
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_generation_matches_expected_distribution() {
        let mut ships = 0;
        for i in 0..500 {
            let mut r = Legacy::large_feature(12345, i * 20, -i * 13);
            let rot = r.next_int_bounded(4) as u8;
            if has_ship(rot, &mut r) {
                ships += 1;
            }
        }
        // Vanilla End Cities generate ships roughly 50-65% of the time.
        assert!((200..=360).contains(&ships), "unexpected ship count {ships}/500");
    }
}
