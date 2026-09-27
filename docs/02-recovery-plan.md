# M0 — Recovery Plan (Partition Backup & Restore)

> **This is the safety gate for the entire project.** No boot-affecting change is
> permitted until M0 is complete and its backups are verified restorable. Enforces
> Rules 1, 2, 3, and 4.

The TicWatch C2 has an **unlocked bootloader**, but per Rule 3 an unlocked bootloader
does **not** guarantee brick recovery. The primary bootloader chain (`sbl`/`xbl`/`tz`)
is typically *not* rewritable via fastboot; corrupting it can hard-brick the device.
Our entire on-device strategy therefore starts from **volatile boot** (`fastboot boot`,
never `fastboot flash`) and full raw backups.

---

## 0. Preconditions

- [ ] Host has `fastboot` + `adb` (Android platform-tools) installed and talking to the
      device.
- [ ] Device charged > 60% before any boot experiment.
- [ ] AsteroidOS **and** Void Linux confirmed booting normally (Rule 1 baseline).
- [ ] Partition table extracted into `docs/01-hardware-map.md` §11.

## 1. Enumerate every partition

From a booted Void Linux (or AsteroidOS) shell:

```sh
# Names → block devices
ls -l /dev/block/bootdevice/by-name/     # or /dev/block/platform/.../by-name/
cat /proc/partitions
lsblk
# GPT dump (host or device, if sgdisk/parted available)
sgdisk -p /dev/block/mmcblk0    # adjust device
```

Record the complete list in the §11 table. **Do not proceed until every partition is
listed.**

## 2. Raw backup of critical partitions

Back up *raw* images (Rule 2). Pull each to the host and store outside this repo (these
are device firmware dumps — **never commit them**; see `.gitignore`).

```sh
# On device (example — substitute real by-name paths from step 1):
for p in sbl1 rpm tz hyp aboot boot recovery modem system persist splash; do
    dd if=/dev/block/bootdevice/by-name/$p of=/tmp/backup_$p.img bs=4M 2>/dev/null || \
        echo "SKIP/absent: $p"
done
# Then pull to host:
adb pull /tmp/  ./device-backups/$(date +%Y%m%d)/
```

Prefer capturing the **whole eMMC** as well when space allows:

```sh
dd if=/dev/block/mmcblk0 of=/tmp/full_emmc.img bs=8M
```

## 3. Verify the backups

A backup you cannot restore is not a backup.

- [ ] Record `sha256sum` of every image; store the checksum list alongside the dumps.
- [ ] Confirm each image size matches the partition size from step 1.
- [ ] Sanity-check that image files are non-zero and not truncated.

## 4. Document and dry-run the restore path

Write, **but do not execute**, the restore commands for each partition, so the exact
recovery procedure exists before it is ever needed:

```sh
# RESTORE (documented, run only in an actual recovery):
fastboot flash boot     device-backups/<date>/backup_boot.img
fastboot flash recovery device-backups/<date>/backup_recovery.img
# etc.
```

- [ ] Confirm the device enters **fastboot** reliably (key combo documented below).
- [ ] Confirm the device enters **EDL / 9008** mode as a last resort, and note whether a
      signed firehose loader is available for the MSM8909 (research only; do not use
      unless recovering).
- [ ] Document the exact button combos for: power-off, fastboot/bootloader, recovery.

### Button combos (fill from device)

| Action | Combo | Confidence |
|--------|-------|------------|
| Force power off | VERIFY | |
| Enter fastboot / bootloader | VERIFY | |
| Enter recovery | VERIFY | |

## 5. Exit criteria for M0

M0 is **done** only when all of the following are true:

- [ ] Every partition enumerated and recorded in §11.
- [ ] Raw backups captured for all rewritable partitions (+ full eMMC if feasible).
- [ ] Checksums recorded and image sizes validated.
- [ ] Restore procedure written and the fastboot/EDL entry paths confirmed reachable.
- [ ] AsteroidOS + Void Linux still boot (baseline intact — Rule 1).

Until every box is checked, **no `fastboot flash`, no boot experiments** — volatile
`fastboot boot` of a *verified* small test image is the first on-device step, and only
after M4 design review.
