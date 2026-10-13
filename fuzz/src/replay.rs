//! Replay one retained input; a semantic assertion failure exits unsuccessfully.

use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let target = args.next().ok_or("missing target")?;
    let paths: Vec<_> = args.collect();
    if paths.is_empty() {
        return Err("missing input paths".into());
    }
    for path in paths {
        let path = Path::new(&path);
        let mut inputs = if path.is_dir() {
            std::fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![path.to_owned()]
        };
        inputs.sort();
        for input in inputs {
            let data = std::fs::read(&input)?;
            // A panic identifies its exact reproducible input, even within a batch.
            eprintln!("{target}: {}", input.display());
            git_pr_fuzz::run(&target, &data);
        }
    }
    Ok(())
}
