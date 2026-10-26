# KilnOS — far-future exploratory vision

> **Status: speculative hobby-project vision, not a scheduled Kiln milestone.**
> KilnOS does not currently exist. Nothing in this document is a commitment
> for Kiln 1.0, an implementation specification, or permission to divert work
> from the [current roadmap](../../README.md#roadmap).
>
> **Working idea:** *Simple to use. Limitless to make.*

## Why KilnOS?

Modern computing is increasingly built around websites and web applications,
yet desktop operating systems still make a strong distinction between "the
browser" and "real apps". At the same time, installing a new OS is unnecessarily
intimidating for many otherwise competent computer users.

**KilnOS imagines a Linux-based, web-first operating environment where websites
are applications, PWAs are first-class, and the desktop itself is programmable
in Kiln.** It should be approachable even to people unfamiliar with the term
"operating system", without hiding capability from developers.

This is a **far-future dogfooding and research project**, not a pivot away from
Kiln's core mission as a web-first language. The ambition is to explore what an
OS designed *after* the rise of the web might look like.

Two guiding ideas:

- **You should never need to open a terminal, but you should never be
  prevented from opening one.**
- **KilnOS should not ask users to solve technical problems that its software
  could reasonably solve for them.**

## Status and scope boundaries

The following describes the intended direction, not existing functionality.

| Area | Current vision |
| --- | --- |
| Kernel | **Reuse Linux**, not a Kiln-written production kernel |
| Browser engine | **Reuse an existing engine**, likely Chromium via CEF; choice unfinalized |
| KilnOS-specific software | Write the shell, system applications, integration services and as much of the compositor as practical **in Kiln** |
| Hardware | Conventional PCs and Linux-supported tablets first; selective phones/ARM64 eventually |
| UI | Web-native, with Glazes/Embers where suitable, and native platform bridges |
| Apps | Websites, PWAs, Kiln apps, native Linux apps, and Windows apps/games via **Wine and Proton available by default** |
| Safety | Protected, recoverable system by default; developer and advanced-unlocked paths |
| Schedule | **None**. No phase number, version target or delivery promise |

"Written in Kiln" refers to **KilnOS-authored components**, not rewriting
Chromium, Linux, drivers, Wine, Proton or every userspace library. Existing
system components and C-compatible APIs should be reused wherever sensible.
Kiln already produces self-contained Linux executables (including programs
that can run in a `scratch` container), but desktop integration still needs
native interop, process/IPC, graphics and platform APIs.

**Important distinction:** the separate
[freestanding/bare-metal hobby OS idea](../vision.md#freestanding-kiln-bare-metal-and-hobby-operating-systems)
is an independent learning experiment about running Kiln without an OS.
**KilnOS in this document is Linux-based** and deliberately does not require
writing a kernel.

## 1. The web is the application platform

Keep normal tabbed browsing **and** let a website become a standalone
application:

- **Browse:** familiar tabs, navigation, history, bookmarks and address bar.
- **Detach:** open any website in a separate application-like window without
  installing it permanently.
- **Install:** give a website a persistent launcher entry, icon, isolated
  storage/permissions as appropriate, and normal OS window management.
- **PWA first-class support:** honour web app manifests, service workers,
  notifications, supported file associations, install/update semantics and
  offline capabilities; disclose what each app actually supports.
- **Kiln-native web applications:** Glazes/Embers and Kiln services can
  integrate through carefully permissioned OS APIs without requiring sites
  to be written in Kiln.

Installing an ordinary website must **not** magically grant it privileged
system access, guarantee offline operation, or bypass browser origin security.
PWA offline behaviour depends on what the app implements. Preserve sessions
when switching between tabs and app windows **where feasible**, rather than
promising lossless transitions in all cases.

The desktop should expose a unified app launcher, task switcher, notifications,
permission controls and app management. A future catalogue could surface
different app types without expecting the user to understand package systems:

- Websites/PWAs through the embedded browser engine.
- Native Kiln and other Linux programs.
- Windows programs through **Wine**, and compatible games through
  **Proton**, available by default without manual compatibility setup.
- Optionally, compatible Android apps through a system such as Waydroid
  (exploratory, **not** a settled dependency).

Compatibility is never universal: websites can require unavailable browser
features, Windows software can fail under Wine/Proton, and some apps need
proprietary services or hardware.

## 2. A fully programmable desktop, with a safe default

The KilnOS desktop shell is a Kiln application (potentially rendered with
Glazes/Embers), not a sealed interface. Developers should be able to write
or replace:

- Taskbars, docks, launchers, notifications, window decorations and widgets.
- Workspace layouts, themes, interaction patterns and accessibility aids.
- Entire desktop shells, published or shared as **Wares** where that package
  model is appropriate.

For everyone else, ordinary visual customisation should be available through
simple settings, prebuilt themes and previewable components. Nobody should
need to learn Kiln just to move a taskbar.

**Compositor policy:** ship a **stable default Wayland compositor**; allow
advanced developers to replace it, ideally with a Kiln-written compositor
using existing native graphics/Wayland building blocks. Shell replacement is
routine and sandboxable; replacing a privileged compositor is a different,
higher-risk operation.

Safety and recoverability should be designed into the architecture:

- A protected core of session, permission and recovery services independent
  of user shell extensions.
- Capability-scoped desktop extensions; no arbitrary privileged access
  simply because a Ware changes the UI.
- A preview/nested-session path for testing replacement shells or compositors.
- A known-good default shell/compositor and a recovery login or boot path.
- Detection of broken startup/crash loops with a route to restore defaults.

A recovery service cannot guarantee safety against unrestricted privileged
modifications, kernel failures or broken drivers. Developer freedom must be
paired with explicit warnings and real fallback paths.

## 3. Adaptive shells: one OS, many experiences

**No separate desktop, tablet and phone products by default.** KilnOS should
share its core application platform and system services across devices, while
choosing a suitable shell based on the hardware and user preferences.

Envisioned modes:

| Context | Possible shell |
| --- | --- |
| Laptop/desktop, keyboard and pointer | Standard multitasking, windows, taskbar |
| Touchscreen/tablet, keyboard detached | Larger controls, gestures, touch keyboard, touch-oriented multitasking |
| Docked/external monitor | Multi-display workspaces, optional tiling and saved layouts |
| Game controller/handheld | Controller-navigable game launcher and Proton integration |
| Compatible phone | Mobile shell with touch-first navigation, calls and notifications |

For example, detaching a Surface keyboard could switch to tablet mode;
attaching a monitor could restore a multi-window workspace; connecting a
controller could *offer* gaming mode. Users must be able to disable automatic
switching and choose a shell manually. Avoid disrupting active work: preserve
applications and layouts where the compositor and application permit it.

A much later convergence experiment could let a supported phone become a
desktop when docked, running the same OS and apps with a different shell.

## 4. Linux foundation, hardware profiles and developer freedom

KilnOS would run on the **Linux kernel**, using existing drivers, graphics,
networking, audio and system facilities rather than reimplementing them.
It should not be tightly coupled to one kernel package.

Hardware-aware profiles could select a suitable kernel, firmware, drivers
and defaults. A supported Microsoft Surface model, for example, could be
offered the **linux-surface** kernel and required supporting services
automatically; exact device support and Secure Boot compatibility must be
verified, not inferred from the Surface brand alone. Keep bootable fallback
kernel options.

**Protected by default, unlocked by choice:**

1. **Standard:** protected/atomic or comparably recoverable OS image,
   graphical updates, rollback and trusted app permissions.
2. **Developer:** real terminal, compilers, containers, Git, debugging,
   system inspection and normal user-level native tools **without needing
   to disable system protection**.
3. **Advanced system unlock:** explicitly opt into lower-level mutable
   host changes, alternate kernels and experimental system components,
   with strong warnings and recovery plans.

The exact base distribution, image/update mechanism, packaging model and
bootloader are open decisions. "Immutable" must not mean preventing developers
from building native software or inspecting their own machines.

## 5. Installation should start on the website

A major KilnOS goal is reducing the adoption barrier created by unfamiliar
installation procedures. The website is part of the product, not merely a
place to download an ISO.

A proposed end-to-end experience:

1. **Explain before asking.** The site includes **"What is an operating
   system?"**, plain-language descriptions of KilnOS, "Can I keep Windows?",
   "What happens to my files?" and "Can I go back?". No condescending tone.
2. **Identify the device.** Detect what a permissioned native helper can
   safely identify, or let the user choose manufacturer/exact model. Show
   tested hardware support and known limitations, not false certainty.
3. **Choose a safe goal.** Try KilnOS from USB, install alongside an
   existing OS where supported, or replace it. Favour trying before erasing.
4. **Prepare the USB.** State the image-specific minimum capacity, ask for
   a removable drive, clearly warn that it will be erased, and verify the
   written image.
5. **Guide boot setup.** Give illustrated, exact-model-aware steps for the
   startup key/USB boot process, with phone-friendly instructions or a QR
   code for use while the computer restarts. Explain firmware concepts in
   ordinary language instead of assuming knowledge of "BIOS" or "UEFI".
6. **Try the live environment.** Test Wi-Fi, display, touch, audio and
   storage before committing to installation where feasible.
7. **Finish on the device.** Guide storage choices, account creation,
   migration options, privacy settings, recovery and desktop setup.
   Destructive changes require an explicit final confirmation.

**Web flashing caveat:** Chrome's Web Serial API is not a raw USB storage
writer; normal browser WebUSB access also does not provide unrestricted
USB mass-storage imaging. A small signed **Kiln-written native setup helper**
could bridge hardware discovery, privileged disk imaging and verification,
with the site remaining the friendly guide. Offer an ordinary verified image
download as an alternative.

The helper must minimise privileges, never silently choose an internal disk,
validate signed images/metadata, and require the user to confirm exactly
which drive will be erased. Setup should account for existing disk
encryption, BitLocker recovery keys, Secure Boot, dual-boot constraints,
backups and cases the installer cannot safely automate. Preserve the user's
data unless they explicitly consent to deleting it.

A future **migration assistant** could offer to bring across accessible
documents, photos, bookmarks and supported settings, while explaining what
cannot transfer. Apps that are simply websites may be easy to reinstall as
PWAs, but login data and proprietary app data may require separate steps.

Experienced users should be able to bypass the wizard and obtain images,
checksums, compatibility information and technical instructions directly.

## 6. Tablets, ARM64 and phones

The ambition is one OS, not just one desktop layout. A realistic exploration
order is x86-64 PC -> Surface/2-in-1 tablet -> ARM64 Linux hardware ->
a specifically supported Linux phone -> docked/mobile convergence.

Android's use of Linux **does not** imply that arbitrary Android phones can
run KilnOS. Locked bootloaders, proprietary GPU/camera drivers, telephony,
suspend/wake reliability, battery life, safety-critical calling and update
mechanisms are major constraints. Target existing well-supported mobile
Linux hardware before attempting broad phone compatibility.

Chromium/CEF integration must also be assessed for mobile memory,
acceleration, battery and ARM64 support. Existing Linux mobile services and
drivers should be reused where appropriate.

## 7. Why this belongs in Kiln's vision

KilnOS would be a demanding real-world application of the language's
native capabilities and web UI system, stretching it beyond HTTP servers:

- Native C ABI/FFI and bindings for browser, Wayland and Linux APIs.
- Processes, signals, IPC, filesystem integration and system services.
- Concurrency, robust error reporting, startup and recovery.
- Graphics, input, device events, accessibility and audio bridges.
- Permissions, isolation, packaging, reproducible updates and hardware
  awareness.
- Portable builds, particularly ARM64, and an embeddable browser UI.

These are **potential requirements to investigate if KilnOS is ever
undertaken**, not requests to add them prematurely to Kiln's current phases.
The aim would be to dogfood Kiln by writing KilnOS software **in Kiln**,
while relying on maintained, proven external infrastructure.

## 8. Possible experimental sequence (not a roadmap)

1. Prototype a Kiln-written launcher/PWA shell **as an app on an existing
   Linux desktop**; do not initially build a distribution.
2. Add install/detach flows, window and app management, and native system
   integration through stable bindings.
3. Explore a Kiln-written shell over an existing Wayland compositor, then
   a replaceable default compositor only when justified.
4. Create a bootable live image, tested recovery path and guided installer.
5. Extend device profiles for tablets, then investigate ARM64/mobile
   targets on known-compatible hardware.

No dates, acceptance gates, phase numbers or compatibility guarantees are
attached to these experiments.

## Deliberate non-goals and open questions

**Non-goals:** rewrite the Linux kernel or Chromium; require every app to be
written in Kiln; sacrifice developer access for a locked-down experience;
claim universal Windows/Android compatibility; or make this OS a prerequisite
for Kiln's web-language success.

**To research much later:** the Linux base and atomic update strategy; how
CEF/Chromium should be embedded and sandboxed; native graphics/compositor
bindings; distribution and licensing implications; PWA storage/update
behaviour; system-level permissions and app-store trust; accessibility;
reliable rollback; hardware certification; and mobile power/telephony.

**In short:** KilnOS is a deliberately ambitious **future hobby OS vision**:
websites as apps, PWAs first-class, a desktop fully programmable in Kiln, and
an installation and day-to-day experience welcoming to people who have never
installed an operating system before. It should remain an inspiration, not a
source of unscheduled work on the Kiln compiler.
