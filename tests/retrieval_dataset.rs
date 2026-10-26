use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[test]
fn container_queries_are_paired_and_task_disjoint() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/retrieval_bkndb/corpus.json")).unwrap();
    let data: Value = serde_json::from_str(include_str!(
        "fixtures/retrieval_bkndb/container-queries.json"
    ))
    .unwrap();
    let mut tasks: BTreeMap<&str, (&str, BTreeSet<&str>)> = BTreeMap::new();
    let queries = data["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 32);
    for q in queries {
        let task = tasks
            .entry(q["task"].as_str().unwrap())
            .or_insert((q["split"].as_str().unwrap(), BTreeSet::new()));
        assert_eq!(task.0, q["split"].as_str().unwrap());
        assert!(task.1.insert(q["query_language"].as_str().unwrap()));
        for label in q["labels"].as_array().unwrap() {
            assert!(
                corpus[label["path"].as_str().unwrap()]
                    .as_str()
                    .unwrap()
                    .contains(label["name"].as_str().unwrap())
            );
        }
    }
    assert!(
        tasks
            .values()
            .all(|(_, langs)| langs == &BTreeSet::from(["en", "id"]))
    );
    assert_eq!(
        tasks
            .values()
            .filter(|(split, _)| *split == "tuning")
            .count(),
        12
    );
    assert_eq!(
        tasks
            .values()
            .filter(|(split, _)| *split == "evaluation")
            .count(),
        4
    );
}
#[test]
fn evaluation_snapshot_labels_and_task_disjoint_splits_are_consistent() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/retrieval_bkndb/corpus.json")).unwrap();
    let manifest: Value =
        serde_json::from_str(include_str!("fixtures/retrieval_bkndb/manifest.json")).unwrap();
    let dataset: Value =
        serde_json::from_str(include_str!("fixtures/retrieval_bkndb/queries.json")).unwrap();
    assert_eq!(
        manifest["revision"],
        "509f9414adc0e312354965b584cd3e165d02a08e"
    );
    assert_eq!(
        corpus.as_object().unwrap().len(),
        manifest["files"].as_object().unwrap().len()
    );
    let mut ids = BTreeSet::new();
    let mut tasks = BTreeMap::new();
    let mut splits = BTreeMap::new();
    let queries = dataset["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 80);
    for query in queries {
        assert!(ids.insert(query["id"].as_str().unwrap()));
        let task = query["task"].as_str().unwrap();
        let split = query["split"].as_str().unwrap();
        assert_eq!(
            *tasks.entry(task).or_insert(split),
            split,
            "task leaks across splits"
        );
        *splits.entry(split).or_insert(0) += 1;
        for label in query["labels"].as_array().unwrap() {
            let source = corpus[label["path"].as_str().unwrap()]
                .as_str()
                .expect("label source in snapshot");
            assert!(source.contains(label["name"].as_str().unwrap()));
            assert!(label["grade"].as_u64().unwrap() <= 3);
        }
    }
    assert_eq!(splits["tuning"], 56);
    assert_eq!(splits["evaluation"], 24);
    #[cfg(feature = "embeddings")]
    {
        use sha2::{Digest, Sha256};
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(include_bytes!("fixtures/retrieval_bkndb/corpus.json"))
            ),
            manifest["corpus_sha256"]
        );
        for (path, source) in corpus.as_object().unwrap() {
            assert_eq!(
                format!("{:x}", Sha256::digest(source.as_str().unwrap().as_bytes())),
                manifest["files"][path]
            );
        }
    }
}
