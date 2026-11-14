//! b8disk — create and edit Bosman-8 floppy images (CP/M 2.2 file system).
//!
//! ```text
//! b8disk new  IMG [--format 200|400|800]
//! b8disk ls   IMG [--format ...]
//! b8disk put  IMG FILE... [--user N] [--format ...]
//! b8disk get  IMG NAME [OUTFILE] [--user N] [--format ...]
//! b8disk rm   IMG NAME... [--user N] [--format ...]
//! ```
//! IMG is a raw sector image or a Gotek `.hfe` file (`new` makes HFE when
//! the name ends in `.hfe`). The format defaults to the one implied by the
//! image size, or for HFE by its sector IDs (200 KB if new).
//! Formats: 200 = default DIP (B:/C:), 400 = DIP bit 3 (`--dip 48`),
//! 800 = DIP bit 5 (`--dip 60`).

use bosman8::cpmfs::{CpmFs, Params};
use bosman8::disk::{self, Format, F200, F400, F800};
use std::path::Path;
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "usage:\n  b8disk new IMG [--format 200|400|800]\n  b8disk ls IMG\n  \
         b8disk put IMG FILE... [--user N]\n  b8disk get IMG NAME [OUTFILE] [--user N]\n  \
         b8disk rm IMG NAME... [--user N]\n\
         IMG: raw sectors or Gotek .hfe\n\
         --format is taken from the image when omitted (default 200)"
    );
    exit(2)
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("b8disk: {msg}");
    exit(1)
}

fn format_by_kb(kb: usize) -> Option<Format> {
    [F200, F400, F800].into_iter().find(|f| f.kbytes() == kb)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut format: Option<Format> = None;
    let mut user = 0u8;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => {
                let v = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                let kb = v.trim_end_matches(['k', 'K']).parse().unwrap_or(0);
                format = Some(format_by_kb(kb).unwrap_or_else(|| fail("format must be 200, 400 or 800")));
                args.drain(i..i + 2);
            }
            "--user" | "-u" => {
                let v = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                user = v.parse().ok().filter(|&u| u < 16).unwrap_or_else(|| fail("user must be 0..15"));
                args.drain(i..i + 2);
            }
            "-h" | "--help" => usage(),
            _ => i += 1,
        }
    }
    if args.len() < 2 {
        usage();
    }
    let cmd = args[0].clone();
    let img_path = args[1].clone();
    let rest = &args[2..];

    if cmd == "new" {
        let f = format.unwrap_or(F200);
        if Path::new(&img_path).exists() {
            fail(format!("{img_path} already exists"));
        }
        std::fs::write(&img_path, disk::blank_file(Path::new(&img_path), f)).unwrap_or_else(|e| fail(e));
        println!("{img_path}: blank {} KB disk", f.kbytes());
        return;
    }

    let file = std::fs::read(&img_path).unwrap_or_else(|e| fail(format!("{img_path}: {e}")));
    let f = format.unwrap_or_else(|| {
        let guess = if bosman8::hfe::is_hfe(&file) {
            bosman8::hfe::Hfe::parse(file.clone())
                .ok()
                .and_then(|h| h.first_sector_id())
                .and_then(disk::format_by_first_sector)
        } else {
            format_by_kb(file.len() / 1024)
        };
        guess.unwrap_or_else(|| fail(format!("{img_path}: unknown disk format, use --format")))
    });
    let opened = disk::open_bytes(file, f).unwrap_or_else(|e| fail(format!("{img_path}: {e}")));
    let hfe_problem = opened.warning.filter(|_| matches!(opened.container, disk::Container::Hfe(_)));
    let mut container = opened.container;
    let mut image = opened.image;
    let mut img = std::mem::take(&mut image.data);
    let mut fs = CpmFs::new(&mut img, Params::for_format(f)).unwrap_or_else(|e| fail(e));
    let mut dirty = false;
    match cmd.as_str() {
        "ls" | "dir" => {
            let files = fs.list();
            for fi in &files {
                println!("{:>2}: {:<12} {:>7}", fi.user, fi.name, fi.size);
            }
            println!("{} file(s), {} KB free of {} KB", files.len(), fs.free_bytes() / 1024, f.kbytes());
        }
        "put" => {
            if rest.is_empty() {
                usage();
            }
            for file in rest {
                let data = std::fs::read(file).unwrap_or_else(|e| fail(format!("{file}: {e}")));
                let name = Path::new(file).file_name().unwrap().to_string_lossy().to_string();
                fs.put(user, &name, &data).unwrap_or_else(|e| fail(e));
                println!("{user}:{}  {} B", name.to_ascii_uppercase(), data.len());
            }
            dirty = true;
        }
        "get" => {
            let name = rest.first().unwrap_or_else(|| usage());
            let data = fs.get(user, name).unwrap_or_else(|e| fail(e));
            let out = rest.get(1).cloned().unwrap_or_else(|| name.to_ascii_uppercase());
            std::fs::write(&out, data).unwrap_or_else(|e| fail(format!("{out}: {e}")));
        }
        "rm" | "era" => {
            for name in rest {
                if !fs.remove(user, name).unwrap_or_else(|e| fail(e)) {
                    fail(format!("{user}:{name}: no such file"));
                }
            }
            dirty = true;
        }
        _ => usage(),
    }
    if dirty {
        if let Some(w) = hfe_problem {
            fail(format!("{img_path}: {w}; not saved"));
        }
        image.data = img;
        std::fs::write(&img_path, container.to_bytes(&image)).unwrap_or_else(|e| fail(e));
    }
}
