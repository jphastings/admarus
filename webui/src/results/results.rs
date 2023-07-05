use std::collections::HashSet;

use crate::prelude::*;

#[derive(Properties, Clone)]
pub struct ResultsPageProps {
    pub app_link: AppLink,
    pub query: Rc<String>,
}

impl PartialEq for ResultsPageProps {
    fn eq(&self, other: &Self) -> bool {
        self.query == other.query
    }
}

pub struct ResultsPage {
    search_data: Option<(u64, Query)>,
    search_error: Option<ApiError>,
    update_counter: u32,
    results: RankedResults,
    providers: HashSet<String>,
}

pub enum ResultsMessage {
    SearchSuccess(ApiSearchResponse),
    SearchFailure(ApiError),
    FetchResultsSuccess(Vec<(DocumentResult, String)>),
    FetchResultsFailure(ApiError),
}

impl Component for ResultsPage {
    type Message = ResultsMessage;
    type Properties = ResultsPageProps;

    fn create(ctx: &Context<Self>) -> Self {
        let query = Rc::clone(&ctx.props().query);
        let link = ctx.link().clone();
        spawn_local(async move {
            match search(query.as_ref()).await {
                Ok(id) => link.send_message(ResultsMessage::SearchSuccess(id)),
                Err(e) => link.send_message(ResultsMessage::SearchFailure(e)),
            }
        });

        Self {
            search_data: None,
            search_error: None,
            update_counter: 0,
            results: RankedResults::new(),
            providers: HashSet::new(),
        }
    }
    
    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            ResultsMessage::SearchSuccess(resp) => {
                let link = ctx.link().clone();
                self.search_data = Some((resp.id, resp.query));
                spawn_local(async move {
                    sleep(Duration::from_millis(100)).await;
                    match fetch_results(resp.id).await {
                        Ok(results) => link.send_message(ResultsMessage::FetchResultsSuccess(results)),
                        Err(e) => link.send_message(ResultsMessage::FetchResultsFailure(e)),
                    }
                });
                false
            }
            ResultsMessage::FetchResultsSuccess(results) => {
                let Some((search_id, query)) = &self.search_data else { return false };
                let search_id = *search_id;

                self.update_counter += 1;
                for (result, provider) in results {
                    self.results.insert(result, provider.clone(), query);
                    self.providers.insert(provider);
                }

                let link = ctx.link().clone();
                let update_counter = self.update_counter;
                spawn_local(async move {
                    match update_counter {
                        0..=10 => sleep(Duration::from_millis(100)).await,
                        11..=20 => sleep(Duration::from_millis(300)).await,
                        _ => sleep(Duration::from_secs(1)).await,
                    }
                    match fetch_results(search_id).await {
                        Ok(results) => link.send_message(ResultsMessage::FetchResultsSuccess(results)),
                        Err(e) => link.send_message(ResultsMessage::FetchResultsFailure(e)),
                    }
                });

                true
            }
            ResultsMessage::SearchFailure(e) | ResultsMessage::FetchResultsFailure(e) => {
                self.search_error = Some(e);
                true
            }
        }
    }

    fn changed(&mut self, ctx: &Context<Self>, _old_props: &Self::Properties) -> bool {
        *self = Component::create(ctx);
        true
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let results = self.results.iter_with_scores().collect::<Vec<_>>();
        let search_id = self.search_data.as_ref().map(|d| d.0);
        let query = self.search_data.as_ref().map(|d| &d.1);

        // General
        let query_string = ctx.props().query.to_string();
        let onsearch = ctx.props().app_link.callback(move |query| AppMsg::ChangePage(Page::Results(Rc::new(query))));
        let onclick_home = ctx.props().app_link.callback(|_| AppMsg::ChangePage(Page::Home));
        let onclick_settings = ctx.props().app_link.callback(|_| AppMsg::ChangePage(Page::Settings));

        // Result counter
        let opt_result_counter = match (results.len(), self.providers.len()) {
            (0, _) => None,
            (1, _) => Some(String::from("1 result")),
            (n, 1) => Some(format!("{} results from 1 provider", n)),
            (n, p) => Some(format!("{} results from {} providers", n, p)),
        };

        // No-results message
        let no_results = results.is_empty() && self.search_error.is_none() && self.update_counter >= 10;
        let many_keywords = ctx.props().query.split_whitespace().count() >= 3;
        let lucky_query = get_lucky_query(search_id);
        let onclick_lucky = ctx.props().app_link.callback(move |_| AppMsg::ChangePage(Page::lucky(search_id)));

        // Error message
        let (opt_error_title, error_recommandations, opt_error_details) = match &self.search_error {
            Some(e) => {
                let (title, recommandations, details) = e.to_format_parts();
                (Some(title), recommandations, Some(details))
            },
            None => (None, Vec::new(), None)
        };
        let error_recommandation_iter = error_recommandations.into_iter();

        // Results
        let addr_iter = results.iter().map(|(result,_)| result.format_best_addr()).collect::<Vec<_>>();
        let title_iter = results.iter().map(|(result,_)| result.format_result_title());
        let description_iter = results.iter().map(|(result,_)| result.view_desc(query.unwrap()));

        // Scores
        let display_scores = true;
        let term_frequency_score_iter = results.iter().map(|(_, scores)| scores.tf_score);
        let length_score_iter = results.iter().map(|(_, scores)| scores.length_score);
        let lang_score_iter = results.iter().map(|(_, scores)| scores.lang_score);
        let popularity_score_iter = results.iter().map(|(_, scores)| scores.popularity_score);

        template_html!(
            "results/results.html",
            onclick_home = { onclick_home.clone() },
            addr_iter = { addr_iter.clone().into_iter() },
            addr2_iter = { addr_iter.iter() },
            ...
        )
    }
}
