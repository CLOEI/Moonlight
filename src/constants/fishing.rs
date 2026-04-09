#[derive(Debug, Clone, Copy)]
pub struct RodProfile {
    pub fill_multiplier: f64,
    pub slider_speed: f64,
    pub slider_size: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct FishBucket {
    pub fish_move_speed: f64,
    pub run_frequency: f64,
    pub min_land_delay: f64,
    pub pull_strength_bamboo: f64,
    pub pull_strength_fiberglass: f64,
    pub pull_strength_carbon: f64,
    pub pull_strength_titanium: f64,
}

pub const MAX_SIZE_MULTIPLIER: f64 = 2.8;
pub const MAX_DIFFICULTY_METER: f64 = 3.1;
pub const DEFAULT_FISH_POSITION: f64 = 0.5;
pub const DEFAULT_TARGET_POSITION: f64 = 0.5;
pub const DEFAULT_PROGRESS: f64 = 0.5;
pub const DEFAULT_DRAG_EXTRA: f64 = 1.0;
pub const FORCE_LAND_EXTRA_DELAY_SECS: f64 = 4.0;
pub const FORCE_LAND_MIN_SECS: f64 = 9.0;
pub const RUN_START_AFTER_SECS: f64 = 5.0;
pub const RUN_DURATION_MS: u64 = 450;
pub const READY_TO_LAND_DELAY_SECS: f64 = 0.15;

pub const FISH_TINY: FishBucket = FishBucket {
    fish_move_speed: 0.60,
    run_frequency: 0.02,
    min_land_delay: 4.2,
    pull_strength_bamboo: 1.7,
    pull_strength_fiberglass: 1.175,
    pull_strength_carbon: 0.775,
    pull_strength_titanium: 0.5,
};

pub const FISH_SMALL: FishBucket = FishBucket {
    fish_move_speed: 0.80,
    run_frequency: 0.04,
    min_land_delay: 4.8,
    pull_strength_bamboo: 3.4,
    pull_strength_fiberglass: 2.35,
    pull_strength_carbon: 1.55,
    pull_strength_titanium: 1.0,
};

pub const FISH_MEDIUM: FishBucket = FishBucket {
    fish_move_speed: 1.25,
    run_frequency: 0.06,
    min_land_delay: 5.5,
    pull_strength_bamboo: 5.1,
    pull_strength_fiberglass: 3.525,
    pull_strength_carbon: 2.325,
    pull_strength_titanium: 1.5,
};

pub const FISH_LARGE: FishBucket = FishBucket {
    fish_move_speed: 1.90,
    run_frequency: 0.08,
    min_land_delay: 6.4,
    pull_strength_bamboo: 6.8,
    pull_strength_fiberglass: 4.7,
    pull_strength_carbon: 3.1,
    pull_strength_titanium: 2.0,
};

pub const FISH_GIANT: FishBucket = FishBucket {
    fish_move_speed: 2.50,
    run_frequency: 0.10,
    min_land_delay: 7.2,
    pull_strength_bamboo: 8.5,
    pull_strength_fiberglass: 5.875,
    pull_strength_carbon: 3.875,
    pull_strength_titanium: 2.5,
};

pub const ROD_DEFAULT: RodProfile = RodProfile {
    fill_multiplier: 1.2,
    slider_speed: 2.0,
    slider_size: 1.0,
};

pub const ROD_FIBERGLASS: RodProfile = RodProfile {
    fill_multiplier: 1.5,
    slider_speed: 2.3,
    slider_size: 1.25,
};

pub const ROD_CARBON: RodProfile = RodProfile {
    fill_multiplier: 2.1,
    slider_speed: 2.9,
    slider_size: 1.5,
};

pub const ROD_TITANIUM: RodProfile = RodProfile {
    fill_multiplier: 1.8,
    slider_speed: 2.6,
    slider_size: 1.8,
};

pub fn rod_profile(rod_block: i32) -> RodProfile {
    match rod_block {
        2407 | 2411 | 2415 | 2419 => ROD_FIBERGLASS,
        2408 | 2412 | 2416 | 2420 | 4196 | 4622 => ROD_CARBON,
        2409 | 2413 | 2417 | 2421 => ROD_TITANIUM,
        _ => ROD_DEFAULT,
    }
}

pub fn fish_bucket_from_name(normalized_name: &str) -> FishBucket {
    if normalized_name.contains("tiny") {
        FISH_TINY
    } else if normalized_name.contains("medium") {
        FISH_MEDIUM
    } else if normalized_name.contains("large") {
        FISH_LARGE
    } else if normalized_name.contains("giant") {
        FISH_GIANT
    } else {
        FISH_SMALL
    }
}

pub fn pull_strength(bucket: FishBucket, rod_family: &str) -> f64 {
    match rod_family {
        "fiberglass" => bucket.pull_strength_fiberglass,
        "carbon" => bucket.pull_strength_carbon,
        "titanium" => bucket.pull_strength_titanium,
        _ => bucket.pull_strength_bamboo,
    }
}
