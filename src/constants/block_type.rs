#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryItemType {
    Block = 0,
    BlockBackground = 1,
    Seed = 2,
    BlockWater = 3,
    WearableItem = 4,
    Weapon = 5,
    Throwable = 6,
    Consumable = 7,
    Shard = 8,
    Blueprint = 9,
    Familiar = 10,
    FAMFood = 11,
    BlockWiring = 12,
}

impl InventoryItemType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Block),
            1 => Some(Self::BlockBackground),
            2 => Some(Self::Seed),
            3 => Some(Self::BlockWater),
            4 => Some(Self::WearableItem),
            5 => Some(Self::Weapon),
            6 => Some(Self::Throwable),
            7 => Some(Self::Consumable),
            8 => Some(Self::Shard),
            9 => Some(Self::Blueprint),
            10 => Some(Self::Familiar),
            11 => Some(Self::FAMFood),
            12 => Some(Self::BlockWiring),
            _ => None,
        }
    }
}
