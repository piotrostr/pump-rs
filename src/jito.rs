use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::Mutex;

use jito_protos::bundle::{bundle_result::Result, BundleResult};
use jito_protos::searcher::searcher_service_client::SearcherServiceClient;
use jito_protos::searcher::SubscribeBundleResultsRequest;
use jito_searcher_client::token_authenticator::ClientInterceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tracing::info;

pub type SearcherClient =
    SearcherServiceClient<InterceptedService<Channel, ClientInterceptor>>;

#[timed::timed(duration(printer = "info!"))]
pub async fn start_bundle_results_listener(
    searcher_client: Arc<Mutex<SearcherClient>>,
) {
    // keep a stream for bundle results
    // TODO hopefully this doesn't deadlock
    let mut bundle_results_stream = searcher_client
        .lock()
        .await
        .subscribe_bundle_results(SubscribeBundleResultsRequest {})
        .await
        .expect("subscribe bundle results")
        .into_inner();
    // poll for bundle results
    tokio::spawn(async move {
        while let Some(res) = bundle_results_stream.next().await {
            if let Ok(BundleResult {
                bundle_id,
                result: Some(result),
            }) = res
            {
                match result {
                    Result::Accepted(_) => {
                        info!("Bundle {} accepted", bundle_id);
                    }
                    Result::Rejected(rejection) => {
                        info!(
                            "Bundle {} rejected: {:?}",
                            bundle_id, rejection
                        );
                    }
                    Result::Dropped(_) => {
                        info!("Bundle {} dropped", bundle_id);
                    }
                    Result::Processed(_) => {
                        info!("Bundle {} processed", bundle_id);
                    }
                    Result::Finalized(_) => {
                        info!("Bundle {} finalized", bundle_id);
                    }
                }
            }
        }
    });
}
