//! `/proc/self/mountinfo` parsing and the choice of which filesystems count as disks.
//!
//! mountinfo rather than `/proc/self/mounts` because it carries the device number: under the
//! hardened unit, `PrivateTmp=`/`StateDirectory=` add bind mounts of `/` (e.g. `/var/tmp`) that
//! `mounts` would show as separate ext4 filesystems.

use super::parse_error;
use crate::CollectError;

/// One line of `/proc/self/mountinfo`, with the mount point unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// `major:minor` of the filesystem; equal for bind mounts and btrfs subvolumes of one fs.
    pub dev: String,
    /// The mount source, e.g. `/dev/sda1`.
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
}

/// Filesystems that never hold user data. Auto-detection skips them (ARCHITECTURE §4).
const PSEUDO_FS: &[&str] = &[
    "autofs",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fuse.gvfsd-fuse",
    "fuse.lxcfs",
    "fuse.portal",
    "fuse.snapfuse",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "nfsd",
    "nsfs",
    "overlay",
    "proc",
    "pstore",
    "ramfs",
    "rpc_pipefs",
    "securityfs",
    "selinuxfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
];

/// Parses `/proc/self/mountinfo`: `id parent major:minor root mount-point options [optional
/// fields...] - fs-type source super-options`.
pub fn parse_mountinfo(text: &str) -> Result<Vec<Mount>, CollectError> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split_ascii_whitespace().collect();
            let separator = fields.iter().skip(6).position(|f| *f == "-").map(|i| i + 6);
            match (fields.get(2), fields.get(4), separator) {
                (Some(dev), Some(point), Some(sep)) if fields.len() > sep + 2 => Ok(Mount {
                    dev: (*dev).to_owned(),
                    device: unescape(fields[sep + 2]),
                    mount_point: unescape(point),
                    fs_type: fields[sep + 1].to_owned(),
                }),
                _ => Err(parse_error(
                    "/proc/self/mountinfo",
                    format!("malformed line {line:?}"),
                )),
            }
        })
        .collect()
}

/// The kernel writes space, tab, newline and backslash in mount fields as 3-digit octal escapes
/// (`\040` and so on). Anything that is not a valid escape is kept as is.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).filter(|d| {
            bytes[i] == b'\\' && d[0] <= b'3' && d.iter().all(|b| (b'0'..=b'7').contains(b))
        });
        if let Some(d) = octal {
            out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Chooses the filesystems to report.
///
/// With `configured` mount points, returns exactly those that are mounted, in config order.
/// Otherwise returns every real filesystem once: a mount point uses its last (topmost) mount,
/// since that is what `statvfs` will see, and of several mounts of one device (bind mounts,
/// btrfs subvolumes) only the shortest mount point is kept.
pub fn select_mounts(mounts: &[Mount], configured: &[String]) -> Vec<Mount> {
    let topmost = |point: &str| {
        mounts
            .iter()
            .rev()
            .find(|m| m.mount_point == point)
            .cloned()
    };
    if !configured.is_empty() {
        return configured.iter().filter_map(|c| topmost(c)).collect();
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut disks = Vec::new();
    for m in mounts {
        if seen.contains(&m.mount_point.as_str()) {
            continue;
        }
        seen.push(&m.mount_point);
        if let Some(top) = topmost(&m.mount_point)
            && !PSEUDO_FS.contains(&top.fs_type.as_str())
        {
            disks.push(top);
        }
    }
    let keep = |i: usize, d: &Mount| {
        !disks.iter().enumerate().any(|(j, o)| {
            o.dev == d.dev
                && (o.mount_point.len() < d.mount_point.len()
                    || (o.mount_point.len() == d.mount_point.len() && j < i))
        })
    };
    disks
        .iter()
        .enumerate()
        .filter(|(i, d)| keep(*i, d))
        .map(|(_, d)| d.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(mounts: &[Mount]) -> Vec<&str> {
        mounts.iter().map(|m| m.mount_point.as_str()).collect()
    }

    #[test]
    fn auto_detects_real_filesystems() {
        let mounts = parse_mountinfo(include_str!(
            "../../tests/fixtures/debian-server/proc/self/mountinfo"
        ))
        .expect("parses");
        let disks = select_mounts(&mounts, &[]);
        assert_eq!(points(&disks), ["/", "/boot/efi", "/srv/music library"]);
        assert_eq!(disks[0].fs_type, "ext4");
        assert_eq!(disks[0].device, "/dev/mapper/vg0-root");
    }

    #[test]
    fn captured_arch_mounts() {
        let mounts = parse_mountinfo(include_str!(
            "../../tests/fixtures/arch-desktop/proc/self/mountinfo"
        ))
        .expect("parses");
        assert_eq!(
            points(&select_mounts(&mounts, &[])),
            ["/", "/boot", "/storage"]
        );
    }

    #[test]
    fn unescapes_octal() {
        let mounts = parse_mountinfo(include_str!(
            "../../tests/fixtures/many-cores/proc/self/mountinfo"
        ))
        .expect("parses");
        assert_eq!(mounts[2].mount_point, "/data set\ttwo\\x");
        assert_eq!(unescape(r"a\012b"), "a\nb");
        assert_eq!(unescape(r"bad\08x\9"), r"bad\08x\9", "invalid escapes kept");
        assert_eq!(unescape(r"end\04"), r"end\04", "truncated escape kept");
    }

    #[test]
    fn configured_mounts_in_config_order_and_missing_skipped() {
        let mounts = parse_mountinfo(include_str!(
            "../../tests/fixtures/debian-server/proc/self/mountinfo"
        ))
        .expect("parses");
        let configured = ["/srv/music library", "/", "/not-mounted", "/tmp"].map(String::from);
        // A configured pseudo filesystem is honoured: the admin asked for it.
        assert_eq!(
            points(&select_mounts(&mounts, &configured)),
            ["/srv/music library", "/", "/tmp"]
        );
    }

    /// Builds a mountinfo line.
    fn line(dev: &str, root: &str, point: &str, fs: &str) -> String {
        format!("1 1 {dev} {root} {point} rw,relatime shared:1 - {fs} /dev/x rw\n")
    }

    #[test]
    fn overmounted_point_reports_topmost_once() {
        let text = [
            line("8:1", "/", "/data", "ext4"),
            line("8:2", "/", "/", "ext4"),
            line("8:3", "/", "/data", "xfs"),
        ]
        .concat();
        let disks = select_mounts(&parse_mountinfo(&text).expect("parses"), &[]);
        assert_eq!(points(&disks), ["/data", "/"]);
        assert_eq!(disks[0].fs_type, "xfs");

        // A real filesystem hidden under a tmpfs is not reported.
        let text = [
            line("8:1", "/", "/x", "ext4"),
            line("0:40", "/", "/x", "tmpfs"),
        ]
        .concat();
        assert!(select_mounts(&parse_mountinfo(&text).expect("ok"), &[]).is_empty());
    }

    #[test]
    fn hardened_unit_bind_mounts_are_not_extra_disks() {
        // The debian-server fixture is seen from inside the unit: PrivateTmp binds /var/tmp and
        // StateDirectory binds /var/lib/gargoyle, both from the root filesystem (253:0).
        let mounts = parse_mountinfo(include_str!(
            "../../tests/fixtures/debian-server/proc/self/mountinfo"
        ))
        .expect("parses");
        assert!(
            mounts
                .iter()
                .any(|m| m.mount_point == "/var/tmp" && m.dev == "253:0")
        );
        let disks = select_mounts(&mounts, &[]);
        assert!(!points(&disks).contains(&"/var/tmp"));
        assert!(!points(&disks).contains(&"/var/lib/gargoyle"));
    }

    #[test]
    fn bind_mount_keeps_the_shortest_mount_point() {
        let text = [
            line("8:17", "/x", "/srv/x", "ext4"),
            line("8:2", "/", "/", "ext4"),
            line("8:17", "/", "/data", "ext4"),
        ]
        .concat();
        let disks = select_mounts(&parse_mountinfo(&text).expect("parses"), &[]);
        assert_eq!(points(&disks), ["/", "/data"]);
    }

    #[test]
    fn btrfs_subvolume_root_is_kept() {
        // `/` is the subvolume /@, so its root field is not "/"; /home is another subvolume of
        // the same filesystem and would report identical usage.
        let text = [
            line("0:31", "/@", "/", "btrfs"),
            line("0:31", "/@home", "/home", "btrfs"),
            line("259:1", "/", "/boot", "vfat"),
        ]
        .concat();
        let disks = select_mounts(&parse_mountinfo(&text).expect("parses"), &[]);
        assert_eq!(points(&disks), ["/", "/boot"]);
    }

    #[test]
    fn configured_mounts_are_not_deduplicated_by_device() {
        let text = [
            line("0:31", "/@", "/", "btrfs"),
            line("0:31", "/@home", "/home", "btrfs"),
        ]
        .concat();
        let mounts = parse_mountinfo(&text).expect("parses");
        assert_eq!(
            points(&select_mounts(&mounts, &["/home".into(), "/".into()])),
            ["/home", "/"]
        );
    }

    #[test]
    fn optional_fields_and_malformed_lines() {
        let m = parse_mountinfo(
            "36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 shared:2 - ext3 /dev/root rw,errors=continue\n",
        )
        .expect("parses");
        assert_eq!(
            m[0],
            Mount {
                dev: "98:0".into(),
                device: "/dev/root".into(),
                mount_point: "/mnt2".into(),
                fs_type: "ext3".into(),
            }
        );
        assert!(parse_mountinfo("1 1 8:1 / /\n").is_err(), "no separator");
        assert!(
            parse_mountinfo("1 1 8:1 / / rw - ext4\n").is_err(),
            "no source"
        );
        assert!(
            parse_mountinfo("/dev/sda1 / ext4 rw 0 0\n").is_err(),
            "mounts format"
        );
    }
}
