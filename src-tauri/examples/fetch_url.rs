//! Dev harness for the URL import path:
//! `cargo run -p soundboard --example fetch_url -- <url> <dest> [from] [to]`
use std::path::PathBuf;
use soundboard_lib::download;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = args.first().cloned().unwrap_or_default();
    let dest = args.get(1).cloned().unwrap_or_else(|| "/tmp/fetch".into());
    let range = match (args.get(2), args.get(3)) {
        (Some(a), Some(b)) => Some((a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0))),
        _ => None,
    };
    download::set_tools_dir(PathBuf::from("/tmp/kakiri-tools"));

    match download::ytdlp() {
        Some((p, v)) => println!("downloader: {} ({v})", p.display()),
        None => println!("downloader: none found"),
    }
    let stale = download::ytdlp().map(|(_, v)| v.as_str() < "2026.").unwrap_or(true);
    if stale {
        println!("fetching latest yt-dlp…");
        match download::ensure_ytdlp(&mut |pct, _| {
            if pct >= 1.0 || (pct * 10.0) as i32 != ((pct - 0.05) * 10.0) as i32 {
                println!("  tool {:.0}%", pct * 100.0);
            }
        }) {
            Ok(v) => println!("installed {v}"),
            Err(e) => {
                println!("FAILED: {e}");
                return;
            }
        }
    }
    match download::probe_page(&url) {
        Ok(i) => println!("probe: \"{}\" ({:.1}s)", i.title, i.duration_secs),
        Err(e) => println!("probe failed: {e}"),
    }
    let got = download::fetch(&url, std::path::Path::new(&dest), range, &mut |pct, stage| {
        if pct >= 1.0 || (pct >= 0.0 && (pct * 20.0) as i32 % 5 == 0) {
            println!("  {stage} {:.0}%", pct * 100.0);
        }
    });
    match got {
        Ok(f) => println!(
            "ok: {} ({:.2}s, {} bytes)",
            f.path,
            f.duration_secs,
            std::fs::metadata(&f.path).map(|m| m.len()).unwrap_or(0)
        ),
        Err(e) => println!("error: {e}"),
    }
}
