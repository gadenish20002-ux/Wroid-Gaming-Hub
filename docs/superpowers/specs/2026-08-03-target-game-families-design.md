# Target Game Families Design

## Goal

Make Wroid recognize and prepare controls for the supported PUBG Mobile and Free Fire editions that may actually be installed by Google Play or sideloading, without overwriting the four canonical starter profiles or any user edits.

## Verified Package Catalog

The catalog includes only editions with current official Google Play pages:

- PUBG Mobile global: `com.tencent.ig`
- PUBG Mobile Korea: `com.pubg.krmobile`
- PUBG Mobile Vietnam: `com.vng.pubgmobile`
- PUBG Mobile Taiwan: `com.rekoo.pubgm`
- Battlegrounds Mobile India: `com.pubg.imobile`
- Free Fire: `com.dts.freefireth`
- Free Fire MAX: `com.dts.freefiremax`
- Brawl Stars: `com.supercell.brawlstars`
- Standoff 2: `com.axlebolt.standoff2`

Package identity is not inferred from prefixes. Every supported package is an explicit catalog entry.

## Architecture

Add a focused `commands/game_catalog.rs` module containing four game families and their explicit variants. Hub presentation, starter ordering, installed-game compatibility, and launch preflight consume this catalog instead of maintaining separate package matches.

The canonical starter remains the editable template for each family. When Hub observes an installed non-canonical variant, it creates a separate derived profile with the variant's exact package and title while cloning the canonical profile's current controls. Existing profiles are detected by package, not filename, and are never overwritten. A user can therefore keep different calibration and control adjustments for global PUBG, PUBG Korea, or Free Fire MAX.

## Reconciliation Flow

1. Load the profile library and collect valid existing package names.
2. Query installed packages only while the Waydroid session is running.
3. For each explicitly supported installed variant, skip canonical variants and variants already represented by any valid profile.
4. Clone the current canonical family profile, change only `name` and `package_name`, validate it, and publish it under the catalog's stable profile id.
5. Publish without replacement. A concurrent refresh or user-created file wins; Wroid removes only its unpublished temporary file.
6. Reload the library only when a new variant profile was created. Any adoption error is reported through existing library errors without hiding the rest of Hub state.

Calibration backgrounds are not copied because an edition can have a different HUD or aspect framing. The derived profile therefore starts with useful controls but visibly remains uncalibrated until the user captures its own reference.

## Compatibility Semantics

The compatibility panel represents four families. A family is installed when any explicit variant is installed and reports the detected package. Launch preflight remains exact: a profile can launch only when its own `package_name` is installed, even if a sibling edition from the same family is present.

Unknown custom packages keep existing behavior and remain usable as custom profiles.

## Security and Persistence

- Profile publication must be atomic and no-replace.
- The profile directory remains user-owned; derived profiles use the existing private profile serializer.
- No package name comes from prefix matching or display text.
- No existing profile, calibration image, or previous-save file is modified.
- Concurrent Hub refreshes must converge to at most one valid derived profile per stable catalog id.

## Tests

- Catalog lookup recognizes every explicit package and rejects near-prefix impostors.
- Family detection prefers the canonical package when multiple variants are installed.
- Compatibility reports an installed regional/Max variant but exact launch preflight rejects a missing sibling.
- Reconciliation creates a valid exact-package clone, preserves bindings, skips existing package profiles, does not overwrite stable-id collisions, and is idempotent.
- Hub JSON uses family kind/description for derived profiles and keeps their independent calibration state.

