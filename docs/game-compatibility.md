# Game Compatibility Setup

Wroid does not hide Android compatibility failures behind a generic launch
error. Check the current guest before installing a game:

```sh
wroid compatibility
```

The Hub shows the same result under **System → Game compatibility**.
Use the setup button there or run:

```sh
wroid compatibility --setup
```

If Waydroid Helper is installed, Wroid opens it directly. On Arch-family
systems with `yay` or `paru`, Wroid opens a terminal containing a visible
installation command for the `waydroid-helper` AUR package. After a successful
install, Wroid opens the helper automatically. The terminal stays open on an
installation error so the cause is not lost. The user still reviews and
confirms the package transaction and privilege prompt. For `yay`, Wroid skips
only the clean-build and diff housekeeping menus; it never passes
`--noconfirm`.

## x86_64 and ARM games

Waydroid's x86_64 image runs x86 Android code directly. Android applications
that ship only ARM native libraries require a native translation component such
as libndk or libhoudini. Wroid detects the guest ABI list and
`ro.dalvik.vm.native.bridge`; it does not download proprietary translation
libraries itself. While Waydroid is stopped, Wroid reads the same saved
properties from `waydroid.cfg`. If neither live nor saved evidence is available,
ARM translation is reported as unknown rather than incorrectly marked missing.

Use a trusted distribution package or one of the projects listed by the
[official Waydroid community resources](https://docs.waydro.id/faq/community-projects-we-like).
After installing the component, restart Waydroid and verify that the report
changes from `ACTION_REQUIRED`:

```sh
waydroid session stop
waydroid session start
wroid compatibility
```

## Google Play

The four starter games are installed by the user from the Play Store. A GAPPS
Waydroid image must expose `com.android.vending`; Wroid checks this without
reading account data. Device certification and Google account sign-in remain
inside Waydroid.

If Waydroid is stopped, the Hub's **Start Waydroid & scan** and **Play Store**
actions start the normal desktop session without sudo, wait until Android's
package manager responds, and refresh package detection. They are disabled by
the backend while a managed game session owns the input bridge.

Install a supported package, then refresh the Hub:

| Game edition | Exact package |
| --- | --- |
| PUBG Mobile global | `com.tencent.ig` |
| PUBG Mobile Korea | `com.pubg.krmobile` |
| PUBG Mobile Vietnam | `com.vng.pubgmobile` |
| PUBG Mobile Taiwan | `com.rekoo.pubgm` |
| Battlegrounds Mobile India | `com.pubg.imobile` |
| Free Fire | `com.dts.freefireth` |
| Free Fire MAX | `com.dts.freefiremax` |
| Brawl Stars | `com.supercell.brawlstars` |
| Standoff 2 | `com.axlebolt.standoff2` |

Package identities are exact; similarly prefixed third-party packages are not
treated as supported games. When a non-canonical supported edition is detected,
Wroid atomically creates a separate profile by copying the current controls
from the canonical family starter and changing only its title and package.
It never overwrites an existing profile, including one stored under another id.
The new edition is launchable immediately but deliberately starts without a
calibration image, because its HUD can differ from the canonical edition.
Open Controls Studio before the first match, choose **Live align**, and select
the actual Waydroid game window. The live game surface stays under the control
markers while they are moved; zoom and pan remove borders or letterboxing, and
**Save aligned frame** retains the calibrated viewport beside the profile.
When the package is installed but no reference exists, the Hub combines the
first two steps in **Open game & calibrate** and changes the game-card status
after the frame is saved.

The same compatibility card measures the host filesystem that backs
`waydroid.host_data_path`; it does not confuse the system-image size with
writable game storage. Wroid recommends at least 40 GiB free for all four games
and their downloadable resources, and marks less than 8 GiB critical. Storage
is advisory for already installed games, so low space never hides an existing
launch action.
