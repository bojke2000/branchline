use branchline::{document::{Command, Document, History, Node, Side}, layout::layout};
use std::{collections::BTreeMap, time::Instant};

fn generated(count: usize) -> Document {
    let mut doc = Document::new("Benchmark");
    let mut ids = vec![doc.root.clone()];
    for index in 1..count {
        let id = format!("node-{index}");
        let parent = ids[(index - 1) / 4].clone();
        let side = if index % 2 == 0 { Side::Left } else { Side::Right };
        doc.nodes.get_mut(&parent).unwrap().children.push(id.clone());
        doc.nodes.insert(id.clone(), Node {
            id: id.clone(), text: format!("Idea {index}"), notes: String::new(), link: String::new(),
            children: vec![], side, folded: false,
        });
        ids.push(id);
    }
    doc
}

fn main() {
    for count in [100, 1_000, 10_000] {
        let mut doc = generated(count);
        let mut history = History::default();
        let sizes = BTreeMap::new();
        let mut times = Vec::new();
        for index in 0..21 {
            let start = Instant::now();
            let root = doc.root.clone();
            history.execute(&mut doc, Command::SetText { id: root, text: format!("Iteration {index}") }).unwrap();
            std::hint::black_box(layout(&doc, &sizes).unwrap());
            times.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        let cold = times.remove(0);
        times.sort_by(f64::total_cmp);
        println!("nodes={count} first_sample_ms={cold:.3} warm_n=20 median_ms={:.3} p95_ms={:.3}", times[10], times[18]);
        if count == 1_000 { assert!(times[18] < 100.0, "1,000-node p95 exceeds 100 ms goal"); }
    }
}
