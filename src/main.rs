#![allow(implied_bounds_entailment)]

mod result;
mod index;
mod prelude;
mod ipfs_rpc;
mod documents;
mod api;
mod kamilata;
mod clap;
mod swarm;
mod discovery;

use crate::prelude::*;


#[tokio::main]
async fn main() {
    let config = Arc::new(Args::parse());

    env_logger::init();

    let index = DocumentIndex::<125000>::new(Arc::clone(&config));
    
    let search_park = Arc::new(SearchPark::new());

    let kamilata = KamilataNode::init(Arc::clone(&config), index.clone()).await;
    let kamilata = kamilata.run();
    if let Some(bootstrap_addr) = &config.kam_bootstrap {
        kamilata.dial(bootstrap_addr.parse().unwrap()).await;
        sleep(Duration::from_secs(2)).await;
        // FIXME: remove this
        todo!("leech from") 
    }

    let f1 = serve_api(&config.api_addr, index.clone(), search_park, kamilata.clone());
    let f2 = index.run();
    let f3 = manage_swarm(kamilata.clone(), Arc::clone(&config));
    let f4 = bootstrap_from_ipfs(kamilata.clone(), Arc::clone(&config));
    let f5 = cleanup_known_peers(kamilata.clone());
    tokio::join!(f1, f2, f3, f4, f5);
}
