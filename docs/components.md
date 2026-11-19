# Components

Drivers, services, apps and compatibility personalities are all
**components**: independently built images with a manifest, started by the
component manager with exactly the capabilities a boot composition grants.
The policy logic lives in `flare-cm` (`component-manager/`), which is
`no_std` so it can run inside the user-space component manager in M2; today
it runs at build time and in `cargo xtask`.

## Manifest (`component.toml`)

```toml
[component]
name = "uart-pl011"
version = "0.1.0"
kind = "driver"          # driver | service | app | personality
swap = "restart"         # live | restart | reset (default: restart)

[binds]                  # drivers only: device compatible strings
compatible = ["arm,pl011"]

[provides]
interfaces = ["flare.console@1.0"]

[requires]
interfaces = []

[lifecycle]              # hooks the component implements
quiesce = false
checkpoint = false
```

* Interfaces are **versioned** `name@MAJOR.MINOR`. A provider of `X@M.n`
  satisfies a requirement for `X@M.m` when `n >= m`.
* `swap` declares what replacement this component supports (spec §6):
  `live` (no visible disruption; needs the `quiesce` hook), `restart`
  (service interruption, clients reconnect) or `reset` (only after a
  hardware reset).
* Manifests are strict: unknown keys are errors.

## Boot composition (`system/*.boot.toml`)

Names the board, the components to start and **every capability** each
receives:

```toml
name = "qemu-virt-phone-demo"
board = "qemu/virt-phone"

[component.uart]
manifest = "drivers/common/uart-pl011"
grants = ["uart=mmio:uart0", "uart-irq=irq:uart0",
          "console=endpoint:console:recv", "klog=klog:read"]
restart = "on-failure"
max-restarts = 3
```

Grant syntax: `<local-name>=<kind>:<target>[:<rights>][:badge=<n>]` with
kinds `mmio:<device>[#n]`, `irq:<device>[#n]`, `endpoint:<name>:<rights>`
and `klog:<rights>`. Device names refer to the board's FHD.

Validation (`flare_cm::boot::validate`, run by the kernel build) rejects:

* grants naming unknown or disabled devices, register windows or interrupts
* a device's registers or interrupt granted to two components (hardware
  ownership is exclusive), device registers granted to non-drivers, or a
  driver granted a device it does not bind
* endpoints with no receiver or no sender; badges on receive capabilities
* more than one kernel-log reader
* unresolved, ambiguous or cyclic interface requirements

## Lifecycle

`flare_cm::lifecycle` encodes the spec's lifecycle as a checked state
machine:

```
Discovered → Resolved → Authorized → Starting → Running ⇄ Quiescing/Quiesced
Running → Stopping → Stopped;  any live state → Failed
Failed/Stopped → (Restart) → Authorized;  → Retired
```

The M1 kernel implements the restart path for real: on a fault or non-zero
exit, a component with `restart = "on-failure"` is torn down (capabilities
revoked, memory freed) and re-created with fresh grants and an incremented
incarnation, up to `max-restarts`; then it is retired.

## Resolution, binding and hot-swap

* `flare_cm::resolve` wires each requirement to exactly one provider
  (ambiguity is an error, never a silent choice), computes start order and
  the dependents of any component.
* `flare_cm::binding` matches FHD devices to drivers using the device's
  *most specific* compatible string first.
* `flare_cm::swap::plan_replacement` produces the ordered steps for
  replacing a component: quiesce clients, stop or quiesce the old one,
  revoke or transfer its capabilities, reset hardware if required, start
  the new one, restore state if both sides checkpoint, reconnect and resume
  clients, retire the old one. The swap is as disruptive as the less capable
  of the two components; it is refused if a current client would lose an
  interface, if the new component's requirements cannot be met, or if a
  replacement driver does not bind the same hardware. **Only direct clients
  are touched** — transitive dependents are not restarted.

Try it: `cargo xtask cm resolve`, `cargo xtask cm swap uart drivers/common/uart-pl011`.
