# Target Game Families Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recognize official PUBG Mobile and Free Fire editions and create independent controls profiles for the exact installed package.

**Architecture:** Centralize explicit package identities in a CLI game catalog. Compatibility consumes family-level detection, while Hub atomically derives a no-overwrite exact-package profile from the user's current canonical controls.

**Tech Stack:** Rust, serde JSON, Waydroid package listing, ProfileV2 atomic persistence, existing Hub local API.

## Global Constraints

- Package identity is matched only against explicit catalog entries, never prefixes.
- Exact-package launch preflight remains mandatory even when a sibling edition is installed.
- Existing profiles, calibration images, and previous-save files are never overwritten.
- Derived editions get independent calibration state.
- Implement every behavior through a red-green TDD cycle.
- Run Rust commands with `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216`.

---

### Task 1: Explicit target-game catalog

**Files:**
- Create: `crates/wroid-cli/src/commands/game_catalog.rs`
- Modify: `crates/wroid-cli/src/commands/mod.rs`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Produces: `GAME_FAMILIES`, `family_for_package(&str)`, `variant_for_package(&str)`, `installed_variant(&GameFamily, &[String])`, and catalog-backed Hub kind/description/order lookups.

- [ ] **Step 1: Write failing catalog behavior tests**

Add literal table tests for all nine verified package ids, rejection of `com.tencent.ig.fake` and `com.dts.freefiremax.clone`, canonical preference when canonical and regional variants coexist, and Hub kind/order for alias profiles.

- [ ] **Step 2: Run focused tests red**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli game_catalog` and confirm failure because the catalog module and alias lookups do not exist.

- [ ] **Step 3: Implement the catalog and replace Hub package matches**

Define immutable family/variant records with stable profile ids and exact packages. Route `starter_order`, `game_kind`, and `game_description` through `family_for_package`; unknown packages retain custom behavior.

- [ ] **Step 4: Run focused tests green**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli game_catalog` and `cargo test -p wroid-cli hub::tests`; require all selected tests to pass without warnings.

### Task 2: Family-aware compatibility with exact launch safety

**Files:**
- Modify: `crates/wroid-cli/src/commands/compatibility.rs`

**Interfaces:**
- Consumes: `GAME_FAMILIES`, `family_for_package`, and `installed_variant`.
- Produces: four family compatibility rows with optional detected package, plus exact `ensure_package_installed_if_known(&str)` behavior.

- [ ] **Step 1: Write failing compatibility tests**

Add tests where only `com.pubg.krmobile` or `com.dts.freefiremax` is installed. Assert the family row is installed and names that package; assert exact preflight accepts that profile and rejects the absent global sibling. Add a near-prefix package case that stays unrecognized.

- [ ] **Step 2: Run focused tests red**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli compatibility::tests` and confirm current canonical-only detection fails the new assertions.

- [ ] **Step 3: Implement family rows and exact package retention**

Store the optional installed package list in `CompatibilityReport`, add `installed_package` to each family row and JSON, use catalog lookup for `game(&str)`, and make launch preflight test the requested exact package rather than the family aggregate.

- [ ] **Step 4: Run focused tests green**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli compatibility::tests` and require all compatibility tests to pass.

### Task 3: Non-destructive installed-variant profile adoption

**Files:**
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Consumes: catalog variant metadata, valid `LibraryProfile` values, and installed Waydroid package ids.
- Produces: `reconcile_installed_game_variants(&Path, &[LibraryProfile], &[String]) -> VariantSyncReport` and atomic no-replace profile publication.

- [ ] **Step 1: Write failing reconciliation tests**

Use real temporary profile directories. Assert an installed PUBG Korea package creates the stable derived id with exact name/package and identical bindings; a second call is idempotent; any existing profile for that package suppresses creation; and a pre-existing stable-id file with unrelated bytes is preserved byte-for-byte.

- [ ] **Step 2: Run focused tests red**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli hub::tests::installed_variant` and confirm the reconciliation API is missing.

- [ ] **Step 3: Implement atomic no-replace reconciliation**

Clone only from the valid canonical family profile, change `name` and `package_name`, validate, save to a unique same-directory temporary path, publish with `fs::hard_link` so an existing destination wins atomically, then remove the temporary link. Return created ids and bounded warnings.

- [ ] **Step 4: Integrate reconciliation into Hub state**

After a successful running-Waydroid package query, reconcile variants and reload the library only when profiles were created. Append warnings to `libraryErrors`; derived profiles naturally use their own calibration sidecars.

- [ ] **Step 5: Run Hub and CLI tests green**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli hub::tests` and `cargo test -p wroid-cli`; require all tests to pass.

### Task 4: Documentation and release verification

**Files:**
- Modify: `README.md`
- Modify: `SPEC.md`
- Modify: `docs/game-compatibility.md`
- Modify: `docs/roadmap.md`

**Interfaces:**
- Consumes: the completed catalog, compatibility, and reconciliation behavior.
- Produces: accurate supported-edition documentation and installed release artifacts.

- [ ] **Step 1: Update product documentation**

List supported exact package variants, explain automatic no-overwrite derived controls profiles, and document that every edition requires its own calibration.

- [ ] **Step 2: Run the full quality gate**

Run workspace tests, strict workspace Clippy, formatting check, Hub/Editor JavaScript syntax checks, and `git diff --check`.

- [ ] **Step 3: Build and install release artifacts**

Build release `wroid`, `wroidd`, and `wroid-helper`; run `wroid desktop install`; verify build/install SHA-256 equality without installing the root helper.

- [ ] **Step 4: Audit stopped runtime state**

Verify Waydroid remains stopped and no Wroid, daemon, helper, Chromium, or temporary variant-test process remains.

