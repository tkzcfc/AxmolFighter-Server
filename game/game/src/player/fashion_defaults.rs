/// 与客户端 FashionDefaults / FashionPosition 对齐的创角外观表。

pub const POS_CLOTHES: i32 = 2;
pub const POS_HAIR: i32 = 4;
pub const POS_SKIN: i32 = 7;

const CLASS_COUNT: usize = 4;
const VARIANT_COUNT: usize = 5;

const HAIR_IDS: [[i32; VARIANT_COUNT]; CLASS_COUNT] = [
    [101004, 101044, 101054, 101064, 101074],
    [102004, 102044, 102054, 102064, 102074],
    [32000, 32001, 32002, 32004, 32005],
    [104004, 104044, 104054, 104064, 104074],
];

const CLOTHES_IDS: [[i32; VARIANT_COUNT]; CLASS_COUNT] = [
    [101002, 101042, 101052, 101062, 101072],
    [102002, 102042, 102052, 102062, 102072],
    [31000, 31001, 31002, 31004, 31005],
    [104002, 104042, 104052, 104062, 104072],
];

const SKIN_IDS: [[i32; VARIANT_COUNT]; CLASS_COUNT] = [
    [101007, 101047, 101057, 101067, 101077],
    [102007, 102047, 102057, 102067, 102077],
    [37000, 37001, 37002, 37004, 37005],
    [104007, 104047, 104057, 104067, 104077],
];

pub struct CreateAppearance {
    pub hair_id: i32,
    pub clothes_id: i32,
    pub skin_id: i32,
}

fn class_index(class_id: i32) -> Option<usize> {
    let i = class_id as isize - 1;
    if i >= 0 && (i as usize) < CLASS_COUNT {
        Some(i as usize)
    } else {
        None
    }
}

fn find_variant(ids: &[i32; VARIANT_COUNT], id: i32) -> Option<usize> {
    ids.iter().position(|v| *v == id)
}

/// 校验创角头发/衣服 id，并按衣服方案下标得到皮肤 id。
pub fn resolve_create_appearance(
    class_id: i32,
    hair_id: i32,
    clothes_id: i32,
) -> Option<CreateAppearance> {
    let idx = class_index(class_id)?;
    if find_variant(&HAIR_IDS[idx], hair_id).is_none() {
        return None;
    }
    let clothes_variant = find_variant(&CLOTHES_IDS[idx], clothes_id)?;
    Some(CreateAppearance {
        hair_id,
        clothes_id,
        skin_id: SKIN_IDS[idx][clothes_variant],
    })
}
