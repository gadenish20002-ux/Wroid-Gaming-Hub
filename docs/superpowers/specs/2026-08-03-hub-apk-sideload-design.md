# Hub APK Sideload Design

## Outcome

The Wroid Hub accepts one local `.apk`, inspects it before installation, reports format and ABI compatibility, and installs it into Waydroid without a terminal. Browser input never selects an arbitrary server-side path.

## Architecture

The loopback HTTP server parses request headers before reading a body. Normal JSON routes keep the 2 MiB body limit. The authenticated APK upload route requires an exact `Content-Length`, accepts at most 4 GiB, streams bytes into a mode-0600 file inside a mode-0700 Wroid state directory, and derives all filenames from a cryptographically random ticket. It never uses the browser filename as a path.

After the stream completes, Wroid runs the existing bounded Android package inspector. Only a single APK with `AndroidManifest.xml`, no encrypted entries, and no confirmed ABI incompatibility is installable. Bundles, OBB files, malformed archives, and confirmed incompatibilities are rejected and removed immediately. Unknown compatibility remains visible but does not block a valid universal or bytecode-only APK.

The browser confirms installation using only the ticket. Hub starts a hidden detached Wroid worker. The worker resolves the ticket inside the private state directory, re-inspects the artifact, waits for desktop Waydroid and its package manager, installs through the Waydroid backend, atomically records a bounded JSON outcome, and removes the staged APK. A status endpoint reads only ticket-derived records. Stale artifacts are cleaned after 24 hours.

## Interface

The library heading gains a secondary `SIDELOAD APK` action. Selecting a file opens an inline industrial “PACKAGE INTAKE” strip above the game deck. `XMLHttpRequest` exposes upload progress. The strip shows filename, size, package format, native ABI set, and Waydroid compatibility, then offers `INSTALL TO WAYDROID` or `DISCARD`. Installation polls status and refreshes the game library when complete. It is keyboard accessible, responsive, and respects reduced motion.

## Security and failure handling

- Bind remains loopback-only and every state-changing endpoint requires the per-process Hub token before accepting a body.
- Uploads require a nonzero exact length, reject transfer encoding, stop at 4 GiB, use `O_NOFOLLOW`, and never overwrite an existing ticket.
- Tickets are 192-bit lowercase hexadecimal values and are validated before filesystem resolution.
- Worker and status files are private and atomically replaced; errors are bounded before serialization.
- Discard removes only the validated ticket artifact. Stale cleanup only touches known ticket suffixes under the private sideload directory.
- One install worker runs at a time through the existing desktop action lease; concurrent attempts return a useful conflict.

## Verification

Rust tests cover header/body separation, authorization before upload, exact-length streaming, size and transfer-encoding rejection, ticket validation, private paths and permissions, preflight rejection, status transitions, worker arguments, cleanup, and API routing. JavaScript checks cover required intake DOM bindings and state transitions. Manual browser verification covers desktop and narrow layouts, progress, discard, failure, and successful installation against a real APK.
