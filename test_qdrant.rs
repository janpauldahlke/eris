use qdrant_client::qdrant::{UpsertPointsBuilder, SearchPointsBuilder, PointStruct, VectorParams, Distance};
use std::collections::HashMap;

fn test() {
    let p = PointStruct::new("id".to_string(), vec![1.0], HashMap::new());
    let u = UpsertPointsBuilder::new("coll", vec![p]);
    
    let s = SearchPointsBuilder::new("coll", vec![1.0], 10).with_payload(true);
}
