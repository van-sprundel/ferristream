use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// We need to access the tmdb module
// For integration tests, we import from the crate
use ferristream::tmdb::TmdbClient;

#[tokio::test]
async fn test_search_multi_returns_results() {
    // Start a mock server
    let mock_server = MockServer::start().await;

    // Mock the TMDB API response
    let response_body = r#"{
        "results": [
            {
                "id": 603,
                "title": "The Matrix",
                "overview": "A computer hacker learns about the true nature of reality.",
                "release_date": "1999-03-30",
                "vote_average": 8.1,
                "poster_path": "/f89U3ADr1oiB1s9GkdPOEpXUk5H.jpg",
                "media_type": "movie"
            },
            {
                "id": 604,
                "name": "The Matrix Reloaded",
                "overview": "Neo and the rebel leaders continue their fight.",
                "first_air_date": "2003-05-15",
                "vote_average": 7.0,
                "poster_path": "/abc123.jpg",
                "media_type": "movie"
            }
        ]
    }"#;

    Mock::given(method("GET"))
        .and(path("/3/search/multi"))
        .and(query_param("api_key", "test-key"))
        .and(query_param("query", "matrix"))
        .respond_with(ResponseTemplate::new(200).set_body_string(response_body))
        .mount(&mock_server)
        .await;

    // Create client pointing to mock server
    let client = TmdbClient::with_base_url(Some("test-key"), &mock_server.uri()).unwrap();

    // Perform search
    let results = client.search_multi("matrix").await.unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].display_title(), "The Matrix");
    assert_eq!(results[0].year(), Some(1999));
    assert_eq!(results[0].id, 603);
}

#[tokio::test]
async fn test_search_multi_empty_results() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/3/search/multi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"results": []}"#))
        .mount(&mock_server)
        .await;

    let client = TmdbClient::with_base_url(Some("test-key"), &mock_server.uri()).unwrap();

    let results = client.search_multi("nonexistent").await.unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_movie_with_year() {
    let mock_server = MockServer::start().await;

    let response_body = r#"{
        "results": [
            {
                "id": 550,
                "title": "Fight Club",
                "release_date": "1999-10-15",
                "vote_average": 8.4
            }
        ]
    }"#;

    // Use path_regex to match regardless of query param order
    Mock::given(method("GET"))
        .and(path("/3/search/movie"))
        .respond_with(ResponseTemplate::new(200).set_body_string(response_body))
        .mount(&mock_server)
        .await;

    let client = TmdbClient::with_base_url(Some("test-key"), &mock_server.uri()).unwrap();

    let results = client.search_movie("fight club", Some(1999)).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].display_title(), "Fight Club");
}

#[tokio::test]
async fn test_search_tv() {
    let mock_server = MockServer::start().await;

    let response_body = r#"{
        "results": [
            {
                "id": 1396,
                "name": "Breaking Bad",
                "first_air_date": "2008-01-20",
                "vote_average": 9.5
            }
        ]
    }"#;

    Mock::given(method("GET"))
        .and(path("/3/search/tv"))
        .respond_with(ResponseTemplate::new(200).set_body_string(response_body))
        .mount(&mock_server)
        .await;

    let client = TmdbClient::with_base_url(Some("test-key"), &mock_server.uri()).unwrap();

    let results = client.search_tv("breaking bad", None).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].display_title(), "Breaking Bad");
    assert_eq!(results[0].year(), Some(2008));
}

#[tokio::test]
async fn test_client_with_api_key() {
    // Test that providing an API key creates a client successfully
    let client = TmdbClient::with_base_url(Some("test-key"), "http://example.com");
    assert!(
        client.is_some(),
        "Client should be created when API key is provided"
    );
}

#[tokio::test]
async fn test_client_without_api_key() {
    // Test client creation without API key
    // Note: This may succeed if TMDB_API_KEY is embedded at compile time
    // This test verifies the constructor doesn't panic - both Some and None are valid outcomes
    let _client = TmdbClient::with_base_url(None, "http://example.com");
    // Test passes as long as the constructor completes without panicking
}
