use crate::prelude::*;
use libp2p::PeerId;
use warp::{Filter, http::Response};
use std::{convert::Infallible, net::SocketAddr};

mod bodies;
use bodies::*;

mod endpoints;
use endpoints::*;

struct OngoingSearch {
    results: Vec<(DocumentResult, PeerId)>,
    last_fetch: Instant,
}

pub struct SearchPark {
    searches: RwLock<HashMap<usize, OngoingSearch>>,
}

impl SearchPark {
    pub fn new() -> SearchPark {
        SearchPark {
            searches: RwLock::new(HashMap::new()),
        }
    }

    pub async fn insert(self: Arc<Self>, controller: SearchController) -> usize {
        let id = rand::random();
        self.searches.write().await.insert(id, OngoingSearch {
            results: Vec::new(),
            last_fetch: Instant::now()
        });
        tokio::spawn(async move {
            let mut controller = controller;
            while let Some((document, peer_id)) = controller.recv().await {
                let mut searches = self.searches.write().await;
                let Some(search) = searches.get_mut(&id) else {break};
                search.results.push((document, peer_id));
                if search.last_fetch.elapsed() > Duration::from_secs(60) {
                    searches.remove(&id);
                    trace!("Search {id} expired");
                    break;
                }
            }
        });
        id
    }

    pub async fn fetch_results(self: Arc<Self>, id: usize) -> Option<Vec<(DocumentResult, PeerId)>> {
        let mut searches = self.searches.write().await;
        let OngoingSearch { results, last_fetch } =  searches.get_mut(&id)?;
        *last_fetch = Instant::now();
        Some(std::mem::take(results))
    }
}

pub async fn serve_api<const N: usize>(api_addr: &str, index: DocumentIndex<N>, search_park: Arc<SearchPark>, kamilata: NodeController) {
    let hello_world = warp::path::end().map(|| "Hello, World at root!");

    let local_search = warp::get()
        .and(warp::path("local-search"))
        .and(warp::query::<ApiSearchQuery>())
        .map(move |q: ApiSearchQuery| (q, index.clone()))
        .and_then(local_search);
    
    let search_park2 = Arc::clone(&search_park);
    let search = warp::get()
        .and(warp::path("search"))
        .and(warp::query::<ApiSearchQuery>())
        .map(move |q: ApiSearchQuery| (q, Arc::clone(&search_park2), kamilata.clone()))
        .and_then(search);

    let fetch_result = warp::get()
        .and(warp::path("fetch-results"))
        .and(warp::query::<ApiResultsQuery>())
        .map(move |id: ApiResultsQuery| (id, Arc::clone(&search_park)))
        .and_then(fetch_results);

    let routes = warp::get().and(
        hello_world
            .or(local_search)
            .or(search)
            .or(fetch_result)
    );

    warp::serve(routes).run(api_addr.parse::<SocketAddr>().unwrap()).await;
}
