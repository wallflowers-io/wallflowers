# `library/machine0` — Arc capacity harness

Arc is the OS for Pacific's containerised micro‑datacentre. Today it is **software
only**: there is no Arc hardware yet. This harness **models Arc's running compute
capacity** by standing each Arc on a [machine0](https://machine0.io) persistent cloud
VM — priced by the minute, each with a **static IP** and an authenticated HTTPS/WSS
endpoint at `<vm>.mac0.io`. **One machine0 VM = one self‑contained Arc.**

```
Pacific (iOS) ─selects nearest Arc(s)─►  Arc = a machine0 VM (static IP, <vm>.mac0.io)
                                           ├─ Semaphore MLS relay   (blind people↔people transport)
                                           └─ arc-node              (membership / signup)
```

This tool manages a set of **independent Arcs** (one per region), recorded in `fleet.json`.

## Prerequisites

```sh
curl -LsSf https://machine0.io/install.sh | sh    # installs the `machine0` CLI
machine0 login                                    # browser OAuth → ~/.machine0/auth-token
#   …or, for automation:  export MACHINE0_API_TOKEN=…   (from https://app.machine0.io)
machine0 whoami                                   # confirm you're authenticated
machine0 sizes                                    # sizes / GPUs / per-minute pricing
```

If `machine0` isn't on `PATH`, point the harness at a binary with `MACHINE0_BIN=…`.
`arcfleet` fails loudly if the CLI is missing or you're not authenticated.

## Usage

```sh
# Bring up independent Arcs (regions: us-east|us-west|uk|eu|asia; sizes: `machine0 sizes`).
# Each Arc gets TWO persistent discs, created and attached automatically:
#   <arc>-state  → /data/arc     open tier: id_ed25519, relay.db, pacific.db, models, logs
#   <arc>-kenjin → /data/kenjin  commercial tier: boxoffice.db (ledger), waker.db (push tokens)
./arcfleet up --count 1 --size small --region uk --image ubuntu-24-04-loaded

./arcfleet verify                    # prove both discs really mounted — DO THIS AFTER EVERY up
./arcfleet ls                        # fleet.json + live machine0 state + each Arc's discs
./arcfleet ip arc-01                 # an Arc's static IP
./arcfleet exec all -- uptime        # run a command on every Arc

./arcfleet bootstrap all             # run arc-node-bootstrap.sh on every Arc

./arcfleet snapshot arc-01 arc-loaded   # freeze a golden image
./arcfleet down                          # rm every Arc, release the IPs (asks first).
                                         # The DISCS SURVIVE — that is the point. A later
                                         # `up --prefix arc` remounts them and the Arc keeps
                                         # its identity. `machine0 disks rm` is the real delete.
```

### The discs (why two, and why not docker volumes)

`arcfleet up` creates the discs first and records them into `fleet.json` — without that
record a `down` + `up` silently returns an Arc with no disc, writing `id_ed25519` and
`relay.db` to the ephemeral root filesystem while reporting success. `arcfleet verify` is the
check for exactly that: an attach that did not take leaves a plain directory that *looks*
right until the first reboot, so `verify` asserts `mountpoint` and a real write, not mere
existence — and cross-checks machine0's own `mountPaths` against the manifest.

A disc can also be attached **after** the fact with `machine0 disks attach <disc> <vm> --path
<abs>` — but only to a **RUNNING** VM created with a **MANAGED** key (`machine0 keys new <name>
--type MANAGED`, a keypair machine0 itself holds), because the server SSHes in to do the
in-guest mount. A VM created with a PUBLIC key can never take an attach; its only route to a
disc is `images save` → `rm` → `new --attach`, **which changes the VM's static IP**. Create
Arcs with a managed key unless you have a reason not to.

Platform limits (Beta): 10 discs per account, 5 per VM, 10–16384 GB, one region per disc, one
VM at a time, absolute non-nesting paths, and **not supported on GPU sizes or NixOS** — a GPU
Arc cannot have a disc at all. Discs survive VM destruction but are **not backed up**: `disks
rm` is permanent and nothing else holds a copy.

A docker volume would survive a redeploy but die with the VM, and the Arc's identity dying
with the VM is not a reset — every device that ever tethered refuses to recognise the Arc.
Two discs rather than one so the open and commercial tiers back up, restore and get destroyed
independently: `/data/kenjin` holds the fee ledger and device push tokens, which the blind
relay must never learn. A disc must live in the **same region** as its VM.

`fleet.json` is **git‑ignored** because it names live, billable infrastructure — see
`fleet.example.json` for the shape.

## What `bootstrap` does per Arc

`arc-node-bootstrap.sh` is pushed to the VM (`machine0 sync push`) and run over `machine0
ssh`. If `ARC_RELAY_BIN` points at a compiled `relay` binary it starts the Semaphore MLS
relay; if `ARC_NODE_BIN` points at a compiled `arc-node` it starts membership/signup.

## Layout

| File | Role |
|------|------|
| `arcfleet` | Fleet manager — wraps `machine0`, owns `fleet.json`. Stdlib Python 3, no deps. |
| `arc-node-bootstrap.sh` | Per‑Arc provisioning, run on the VM. |
| `fleet.example.json` | Shape of the generated manifest. |
| `Makefile` | `make up N=1`, `make ls`, `make bootstrap`, `make down`. |
