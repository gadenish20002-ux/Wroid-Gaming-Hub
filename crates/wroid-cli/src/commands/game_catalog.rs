#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GameVariant {
    pub(crate) package: &'static str,
    pub(crate) name: &'static str,
    pub(crate) profile_id: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GameFamily {
    pub(crate) name: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) canonical_package: &'static str,
    pub(crate) description: &'static str,
    pub(crate) order: usize,
    pub(crate) variants: &'static [GameVariant],
}

const PUBG_VARIANTS: &[GameVariant] = &[
    GameVariant {
        package: "com.tencent.ig",
        name: "PUBG Mobile",
        profile_id: "pubg-mobile",
    },
    GameVariant {
        package: "com.pubg.krmobile",
        name: "PUBG Mobile Korea",
        profile_id: "pubg-mobile-korea",
    },
    GameVariant {
        package: "com.vng.pubgmobile",
        name: "PUBG Mobile Vietnam",
        profile_id: "pubg-mobile-vietnam",
    },
    GameVariant {
        package: "com.rekoo.pubgm",
        name: "PUBG Mobile Taiwan",
        profile_id: "pubg-mobile-taiwan",
    },
    GameVariant {
        package: "com.pubg.imobile",
        name: "Battlegrounds Mobile India",
        profile_id: "bgmi",
    },
];

const FREE_FIRE_VARIANTS: &[GameVariant] = &[
    GameVariant {
        package: "com.dts.freefireth",
        name: "Free Fire",
        profile_id: "free-fire",
    },
    GameVariant {
        package: "com.dts.freefiremax",
        name: "Free Fire MAX",
        profile_id: "free-fire-max",
    },
];

const BRAWL_STARS_VARIANTS: &[GameVariant] = &[GameVariant {
    package: "com.supercell.brawlstars",
    name: "Brawl Stars",
    profile_id: "brawl-stars",
}];

const STANDOFF_VARIANTS: &[GameVariant] = &[GameVariant {
    package: "com.axlebolt.standoff2",
    name: "Standoff 2",
    profile_id: "standoff-2",
}];

pub(crate) const GAME_FAMILIES: [GameFamily; 4] = [
    GameFamily {
        name: "PUBG Mobile",
        kind: "pubg",
        canonical_package: "com.tencent.ig",
        description: "Battle royale · keyboard + precision aim",
        order: 0,
        variants: PUBG_VARIANTS,
    },
    GameFamily {
        name: "Free Fire",
        kind: "freefire",
        canonical_package: "com.dts.freefireth",
        description: "Fast battle royale · tuned for low latency",
        order: 1,
        variants: FREE_FIRE_VARIANTS,
    },
    GameFamily {
        name: "Brawl Stars",
        kind: "brawl",
        canonical_package: "com.supercell.brawlstars",
        description: "Twin-stick action · dual virtual joysticks",
        order: 2,
        variants: BRAWL_STARS_VARIANTS,
    },
    GameFamily {
        name: "Standoff 2",
        kind: "standoff",
        canonical_package: "com.axlebolt.standoff2",
        description: "Competitive FPS · mouse aim + keyboard",
        order: 3,
        variants: STANDOFF_VARIANTS,
    },
];

pub(crate) fn family_for_package(package: &str) -> Option<&'static GameFamily> {
    variant_for_package(package)?;
    GAME_FAMILIES.iter().find(|family| {
        family
            .variants
            .iter()
            .any(|variant| variant.package == package)
    })
}

pub(crate) fn variant_for_package(package: &str) -> Option<&'static GameVariant> {
    GAME_FAMILIES
        .iter()
        .flat_map(|family| family.variants)
        .find(|variant| variant.package == package)
}

pub(crate) fn installed_variant<'a>(
    family: &'a GameFamily,
    installed_packages: &[String],
) -> Option<&'a GameVariant> {
    family.variants.iter().find(|variant| {
        installed_packages
            .iter()
            .any(|package| package == variant.package)
    })
}
