use super::*;
use crate::http::{Http, MAX_RESPONSE};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

fn test_bio(base: &str) -> NativeBio {
    let base = base.trim_end_matches('/');
    NativeBio::test_client(
        &[
            ("NCBI_EUTILS_URL".into(), format!("{base}/entrez/eutils")),
            ("GEO_ACC_URL".into(), format!("{base}/geo/query/acc.cgi")),
            (
                "ARRAYEXPRESS_BASE_URL".into(),
                format!("{base}/biostudies/api/v1"),
            ),
            (
                "ARRAYEXPRESS_FILES_URL".into(),
                format!("{base}/biostudies/files"),
            ),
            (
                "METABOLIGHTS_BASE_URL".into(),
                format!("{base}/metabolights/ws"),
            ),
            (
                "MGNIFY_BASE_URL".into(),
                format!("{base}/metagenomics/api/v2"),
            ),
            (
                "PRIDE_BASE_URL".into(),
                format!("{base}/pride/ws/archive/v3"),
            ),
            ("NCBI_API_KEY".into(), "synthetic-key&value".into()),
            ("NCBI_EMAIL".into(), "operator@example.test".into()),
        ],
        Http(reqwest::Client::builder().no_proxy().build().unwrap()),
    )
    .unwrap()
}

async fn serve(app: Router) -> (NativeBio, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (test_bio(&endpoint), task)
}

fn series_soft() -> &'static str {
    "^SERIES = GSE1\n\
     !Series_title = Synthetic GEO series\n\
     !Series_summary = Invented transcriptomic series\n\
     !Series_overall_design = Control versus treatment\n\
     !Series_sample_id = GSM1\n\
     !Series_platform_id = GPL1\n\
     !Series_pubmed_id = 12345678\n\
     !Series_supplementary_file = ftp://example.test/file.txt\n"
}

fn sample_soft() -> &'static str {
    "^SAMPLE = GSM1\n\
     !Sample_title = Synthetic sample\n\
     !Sample_geo_accession = GSM1\n\
     !Sample_organism_ch1 = Homo sapiens\n\
     !Sample_characteristics_ch1 = tissue: liver\n\
     !Sample_library_strategy = RNA-Seq\n\
     !Sample_instrument_model = Synthetic sequencer\n"
}

fn ae_study() -> Value {
    json!({
        "accno": "E-MTAB-1",
        "attributes": [
            {"name": "ReleaseDate", "value": "2020-01-02"},
            {"name": "Title", "value": "Synthetic ArrayExpress experiment"}
        ],
        "section": {
            "type": "Study",
            "attributes": [
                {"name": "Title", "value": "Synthetic ArrayExpress experiment"},
                {"name": "Organism", "value": "Homo sapiens"},
                {"name": "Study type", "value": "RNA-seq of coding RNA"},
                {"name": "Description", "value": "Invented experiment"}
            ],
            "files": [{
                "path": "E-MTAB-1.sdrf.txt",
                "size": 64,
                "attributes": [
                    {"name": "Type", "value": "SDRF File"},
                    {"name": "Format", "value": "tab-delimited text"}
                ]
            }],
            "subsections": [
                {"type": "Samples", "attributes": [
                    {"name": "Sample count", "value": "1"},
                    {"name": "Experimental Factors", "value": "treatment"}
                ]},
                {"type": "Assays and Data", "attributes": [
                    {"name": "Assay count", "value": "1"},
                    {"name": "Technology", "value": "sequencing assay"}
                ]},
                {"type": "Author", "attributes": [{"name": "Name", "value": "Ada Lovelace"}]}
            ]
        }
    })
}

fn archive_app() -> Router {
    Router::new()
        .route(
            "/entrez/eutils/esearch.fcgi",
            post(|| async {
                Json(json!({"esearchresult": {"count": "2", "idlist": ["200000001"]}}))
            }),
        )
        .route(
            "/entrez/eutils/esummary.fcgi",
            post(|| async {
                Json(json!({"result": {
                    "uids": ["200000001"],
                    "200000001": {
                        "uid": "200000001",
                        "accession": "GSE1",
                        "title": "Synthetic GEO series",
                        "summary": "Invented transcriptomic series",
                        "entrytype": "GSE",
                        "gdstype": "Expression profiling by high throughput sequencing",
                        "taxon": "Homo sapiens",
                        "n_samples": 1,
                        "pdat": "2020/01/02",
                        "gpl": "GPL1",
                        "bioproject": "PRJNA1",
                        "ftplink": "ftp://ftp.ncbi.nlm.nih.gov/geo/series/GSE1nnn/GSE1/",
                        "pubmedids": ["12345678"],
                        "samples": [{"accession": "GSM1", "title": "Synthetic sample"}]
                    }
                }}))
            }),
        )
        .route(
            "/geo/query/acc.cgi",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                if q.get("targ").map(String::as_str) == Some("gsm") {
                    sample_soft().to_string()
                } else {
                    series_soft().to_string()
                }
            }),
        )
        .route(
            "/biostudies/api/v1/arrayexpress/search",
            get(|| async {
                Json(json!({
                    "page": 1,
                    "pageSize": 100,
                    "totalHits": 3,
                    "isTotalHitsExact": true,
                    "hits": [
                        {"accession": "E-MTAB-3", "title": "C", "release_date": "2020-03-01", "files": 1, "links": 0, "isPublic": true},
                        {"accession": "E-MTAB-2", "title": "B", "release_date": "2020-02-01", "files": 2, "links": 1, "isPublic": true},
                        {"accession": "E-MTAB-1", "title": "A", "release_date": "2020-01-01", "files": 3, "links": 2, "isPublic": true}
                    ]
                }))
            }),
        )
        .route(
            "/biostudies/api/v1/studies/{acc}",
            get(|Path(acc): Path<String>| async move {
                if acc == "E-MTAB-1" {
                    Json(ae_study()).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/biostudies/api/v1/studies/{acc}/info",
            get(|Path(acc): Path<String>| async move {
                Json(json!({
                    "accno": acc,
                    "files": 1,
                    "httpLink": "https://www.ebi.ac.uk/biostudies/files/E-MTAB-1",
                    "ftpLink": "ftp://ftp.ebi.ac.uk/pub/databases/microarray/data/experiment/MTAB/E-MTAB-1",
                    "relPath": "arrayexpress-repository/E-MTAB-1"
                }))
            }),
        )
        .route(
            "/biostudies/files/{acc}/{*path}",
            get(|| async {
                "Source Name\tCharacteristics [tissue]\tFactor Value [treatment]\n\
                 SAMPLE-1\tliver\tcontrol\n"
            }),
        )
        .route(
            "/metabolights/ws/studies",
            get(|| async {
                Json(json!({"content": ["MTBLS10", "MTBLS2", "MTBLS1"], "studies": 3}))
            }),
        )
        .route(
            "/metabolights/ws/studies/public/study/{acc}",
            get(|Path(acc): Path<String>| async move {
                if acc == "MTBLS1" {
                    Json(json!({
                        "content": {
                            "studyIdentifier": "MTBLS1",
                            "title": "Synthetic metabolomics study",
                            "studyDescription": "Invented ISA payload",
                            "studyStatus": "Public",
                            "organism": [{"organismName": "Homo sapiens", "organismPart": "liver"}],
                            "assays": [{"assayNumber": 1, "measurement": "metabolite profiling",
                                "technology": "mass spectrometry", "platform": "Synthetic MS",
                                "fileName": "a_mtbls1.txt"}],
                            "factors": [{"name": "Treatment"}],
                            "descriptors": [{"description": "EFO:synthetic"}],
                            "protocols": [{"name": "Extraction", "description": "Invented protocol"}],
                            "sampleTable": {
                                "fields": {
                                    "0~Source Name": {"index": 0, "header": "Source Name"},
                                    "1~Characteristics[Organism]": {"index": 1, "header": "Characteristics[Organism]"}
                                },
                                "data": [["S1", "Homo sapiens"], ["S2", "Homo sapiens"], ["S3", "Mus musculus"]]
                            },
                            "derivedData": {"releaseYear": 2020, "submissionYear": 2019}
                        }
                    }))
                    .into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/metabolights/ws/studies/{acc}/files",
            get(|| async {
                Json(json!({
                    "latest": true,
                    "study": [
                        {"file": "i_Investigation.txt", "type": "metadata", "status": "active", "directory": false},
                        {"file": "FILES", "type": "folder", "status": "active", "directory": true}
                    ]
                }))
            }),
        )
        .route(
            "/metabolights/ws/studies/{acc}/public-data-files",
            get(|| async {
                Json(json!({"files": [
                    {"name": "FILES/run1.mzML"},
                    {"name": "FILES/run1.raw"}
                ]}))
            }),
        )
        .route(
            "/metagenomics/api/v2/studies",
            get(|| async {
                Json(json!({
                    "count": 2,
                    "items": [
                        {"accession": "MGYS00000002", "study_name": "Synthetic wastewater",
                            "bioproject": "PRJEB2", "samples_count": 4, "centre_name": "EMBL-EBI",
                            "biomes": ["root:Engineered:Wastewater"]},
                        {"accession": "MGYS00000001", "study_name": "Synthetic gut",
                            "study_abstract": "Invented microbiome study",
                            "bioproject": "PRJEB1", "samples_count": 3, "centre_name": "EMBL-EBI",
                            "secondary_accession": "ERP000001",
                            "biomes": ["root:Host-associated:Human"]}
                    ]
                }))
            }),
        )
        .route(
            "/metagenomics/api/v2/studies/{acc}",
            get(|Path(acc): Path<String>| async move {
                if acc == "MGYS00000001" {
                    Json(json!({
                        "accession": "MGYS00000001",
                        "study_name": "Synthetic gut",
                        "study_abstract": "Invented microbiome study",
                        "bioproject": "PRJEB1",
                        "secondary_accession": "ERP000001",
                        "samples_count": 3,
                        "centre_name": "EMBL-EBI",
                        "biomes": ["root:Host-associated:Human"]
                    }))
                    .into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/metagenomics/api/v2/studies/{acc}/analyses",
            get(|| async {
                Json(json!({
                    "count": 1,
                    "items": [{
                        "accession": "MGYA00000001",
                        "pipeline_version": "5.0",
                        "experiment_type": "metagenomic",
                        "analysis_status": "completed",
                        "run_accession": "ERR1",
                        "sample_accession": "ERS1"
                    }]
                }))
            }),
        )
        .route(
            "/metagenomics/api/v2/biomes/{lineage}/studies",
            get(|| async {
                Json(json!({
                    "count": 1,
                    "items": [{"accession": "MGYS00000001", "study_name": "Synthetic gut",
                        "biomes": ["root:Host-associated:Human"], "samples_count": 3}]
                }))
            }),
        )
        .route(
            "/pride/ws/archive/v3/search/projects",
            get(|| async {
                Json(json!([
                    {"accession": "PXD000002", "title": "Second synthetic proteome",
                        "organisms": ["Mus musculus"], "instruments": ["Synthetic Orbitrap"],
                        "experimentTypes": ["Shotgun proteomics"], "submissionDate": "2020-02-01"},
                    {"accession": "PXD000001", "title": "Synthetic proteome",
                        "organisms": [{"name": "Homo sapiens (human)"}],
                        "instruments": ["Synthetic Orbitrap"],
                        "experimentTypes": ["Shotgun proteomics"],
                        "diseases": ["none"],
                        "submissionDate": "2020-01-01T12:00:00",
                        "submitters": [{"firstName": "Ada", "lastName": "Lovelace"}],
                        "references": ["Invented paper--pubMed:12345678--doi: 10.example/synthetic"]}
                ]))
            }),
        )
        .route(
            "/pride/ws/archive/v3/projects/{acc}",
            get(|Path(acc): Path<String>| async move {
                if acc == "PXD000001" {
                    Json(json!({
                        "accession": "PXD000001",
                        "title": "Synthetic proteome",
                        "projectDescription": "Invented PRIDE project",
                        "organisms": [{"name": "Homo sapiens (human)"}],
                        "instruments": [{"name": "Synthetic Orbitrap"}],
                        "experimentTypes": [{"name": "Shotgun proteomics"}],
                        "submissionDate": "2020-01-01",
                        "pubmedID": "12345678"
                    }))
                    .into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/pride/ws/archive/v3/pride-ap/search/proteins",
            get(|| async {
                Json(json!([
                    {"proteinAccession": "P04637", "proteinName": "Cellular tumor antigen p53",
                        "gene": "TP53", "projectCount": 2}
                ]))
            }),
        )
        .route(
            "/pride/ws/archive/v3/proteins/search",
            get(|| async {
                Json(json!([
                    {"proteinAccession": "P04637", "projects": ["PXD000002", "PXD000001"]}
                ]))
            }),
        )
}

#[test]
fn catalog_registers_seventeen_omics_archive_tools() {
    let names: Vec<_> = catalog()
        .into_iter()
        .map(|(domain, schema)| (domain, schema.function.name))
        .collect();
    assert_eq!(
        names,
        vec![
            ("omics-archives", "arrayexpress_get_experiment".into()),
            ("omics-archives", "arrayexpress_get_experiment_files".into()),
            (
                "omics-archives",
                "arrayexpress_get_experiment_samples".into()
            ),
            ("omics-archives", "arrayexpress_search_experiments".into()),
            ("omics-archives", "geo_get_series".into()),
            ("omics-archives", "geo_search_series".into()),
            ("omics-archives", "metabolights_get_studies".into()),
            ("omics-archives", "metabolights_get_study_files".into()),
            ("omics-archives", "metabolights_list_studies".into()),
            ("omics-archives", "metabolights_search_data_files".into()),
            ("omics-archives", "mgnify_get_studies".into()),
            ("omics-archives", "mgnify_get_study_analyses".into()),
            ("omics-archives", "mgnify_search_studies".into()),
            ("omics-archives", "pride_find_projects_for_protein".into()),
            ("omics-archives", "pride_get_projects".into()),
            ("omics-archives", "pride_search_project_proteins".into()),
            ("omics-archives", "pride_search_projects".into()),
        ]
    );
    assert!(crate::contains_tool("geo_search_series"));
    assert_eq!(
        crate::domain_for_tool("pride_search_projects"),
        Some("omics-archives")
    );
    assert!(crate::package_selects(
        "mcp_omics_archives",
        "omics-archives"
    ));
    assert!(crate::selected_by_package("mcp_omics_archives"));
}

#[test]
fn rejects_unbounded_or_malformed_arguments() {
    assert!(
        serde_json::from_value::<geo::SearchSeries>(json!({"term": "x", "api_key": "s"})).is_err()
    );
    assert!(bound_page(0).is_err());
    assert!(bound_page(201).is_err());
    assert!(unique_ids(&[" ".into()], 20, "id").is_err());
    assert!(unique_ids(&["GSE1,GSE2".into()], 20, "id").is_err());
    assert!(unique_ids(
        &(1..=21).map(|n| format!("GSE{n}")).collect::<Vec<_>>(),
        20,
        "id"
    )
    .is_err());
    assert!(iso_date("2020/01/01").is_err());
    assert!(iso_date("2020-02-30").is_err());
    assert_eq!(iso_date("2020-01-02").unwrap(), "2020-01-02");
    assert!(matches_prefix_digits("gse12", "GSE"));
    assert!(!matches_prefix_digits("GSM12", "GSE"));
}

#[test]
fn parsers_preserve_source_urls_and_sample_rows() {
    let series = geo::parse_series_header(series_soft()).unwrap();
    assert_eq!(series["title"], "Synthetic GEO series");
    assert_eq!(series["platform_ids"], json!(["GPL1"]));
    let samples = geo::parse_sample_headers(sample_soft());
    assert_eq!(samples[0]["characteristics"][0]["tag"], "tissue");
    assert_eq!(samples[0]["characteristics"][0]["value"], "liver");
    let sdrf = arrayexpress::parse_sdrf(
        "Source Name\tCharacteristics [tissue]\tCharacteristics [tissue]\nS1\tliver\ttumor\n",
    )
    .unwrap();
    assert_eq!(
        sdrf.headers,
        vec![
            "Source Name".to_string(),
            "Characteristics [tissue]".to_string(),
            "Characteristics [tissue]#2".to_string()
        ]
    );
    assert_eq!(sdrf.samples[0]["Characteristics [tissue]#2"], "tumor");
    let flat = arrayexpress::flatten_study(&ae_study()).unwrap();
    assert_eq!(flat["accession"], "E-MTAB-1");
    assert_eq!(flat["sample_count"], 1);
    assert_eq!(flat["organisms"], json!(["Homo sapiens"]));
    let table = metabolights::sample_table(
        Some(&json!({
            "fields": {"0~Source Name": {"index": 0, "header": "Source Name"}},
            "data": [["S1"], ["S2"], ["S3"]]
        })),
        2,
    );
    assert_eq!(table["n_rows_total"], 3);
    assert_eq!(table["rows_truncated"], true);
    assert_eq!(table["rows"].as_array().unwrap().len(), 2);
    let pride = pride::project_record(
        &json!({
            "accession": "PXD000001",
            "title": "Synthetic proteome",
            "organisms": [{"name": "Homo sapiens (human)"}],
            "references": ["Invented--pubMed:123--doi: 10.example/x"]
        }),
        "search",
    );
    assert_eq!(
        pride["url"],
        "https://www.ebi.ac.uk/pride/archive/projects/PXD000001"
    );
    assert_eq!(pride["references"][0]["pubmed_id"], "123");
}

#[tokio::test]
async fn each_archive_dispatches_through_native_bio_call() {
    let (bio, server) = serve(archive_app()).await;

    let geo = bio
        .call(
            "geo_search_series",
            &json!({"term": "GSE1[ACCN] AND gse[ETYP]", "retmax": 1}),
        )
        .await
        .unwrap();
    let ae = bio
        .call(
            "arrayexpress_search_experiments",
            &json!({"query": "synthetic", "organism": "Homo sapiens", "max_records": 1}),
        )
        .await
        .unwrap();
    let mtbls = bio
        .call("metabolights_list_studies", &json!({"max_returned": 2}))
        .await
        .unwrap();
    let mgnify = bio
        .call(
            "mgnify_search_studies",
            &json!({"query": "gut", "max_records": 1}),
        )
        .await
        .unwrap();
    let pride = bio
        .call(
            "pride_search_projects",
            &json!({"keyword": "proteome", "max_records_returned": 1}),
        )
        .await
        .unwrap();
    server.abort();

    assert_eq!(geo["source"], "NCBI GEO DataSets");
    assert_eq!(geo["source_url"], "https://www.ncbi.nlm.nih.gov/geo/");
    assert_eq!(geo["total"], 2);
    assert_eq!(geo["returned"], 1);
    assert_eq!(geo["truncated"], true);
    assert_eq!(
        geo["records"][0]["url"],
        "https://www.ncbi.nlm.nih.gov/geo/query/acc.cgi?acc=GSE1"
    );
    assert!(!geo.to_string().contains("synthetic-key"));

    assert_eq!(ae["source"], "ArrayExpress (BioStudies)");
    assert_eq!(
        ae["source_url"],
        "https://www.ebi.ac.uk/biostudies/arrayexpress"
    );
    assert_eq!(ae["total_hits"], 3);
    assert_eq!(ae["returned"], 1);
    assert_eq!(ae["truncated"], true);
    assert_eq!(
        ae["records"][0]["url"],
        "https://www.ebi.ac.uk/biostudies/arrayexpress/studies/E-MTAB-3"
    );

    assert_eq!(mtbls["source"], "MetaboLights");
    assert_eq!(mtbls["reported_count"], 3);
    assert_eq!(mtbls["truncated"], true);
    assert_eq!(mtbls["accessions"], json!(["MTBLS1", "MTBLS2"]));

    assert_eq!(mgnify["source"], "MGnify");
    assert_eq!(mgnify["count"], 2);
    assert_eq!(mgnify["returned"], 1);
    assert_eq!(mgnify["truncated"], true);
    assert_eq!(
        mgnify["records"][0]["url"],
        "https://www.ebi.ac.uk/metagenomics/studies/MGYS00000002"
    );

    assert_eq!(pride["source"], "PRIDE Archive");
    assert_eq!(pride["source_url"], "https://www.ebi.ac.uk/pride/archive");
    assert_eq!(pride["returned"], 1);
    assert_eq!(pride["truncated"], true);
    assert_eq!(
        pride["records"][0]["url"],
        "https://www.ebi.ac.uk/pride/archive/projects/PXD000002"
    );
}

#[tokio::test]
async fn remaining_tools_report_missing_records_and_source_urls() {
    let (bio, server) = serve(archive_app()).await;
    let geo = bio
        .call("geo_get_series", &json!({"accessions": ["GSE1", "GSE9"]}))
        .await
        .unwrap();
    let experiment = bio
        .call(
            "arrayexpress_get_experiment",
            &json!({"accession": "E-MTAB-1"}),
        )
        .await
        .unwrap();
    let files = bio
        .call(
            "arrayexpress_get_experiment_files",
            &json!({"accession": "E-MTAB-1"}),
        )
        .await
        .unwrap();
    let samples = bio
        .call(
            "arrayexpress_get_experiment_samples",
            &json!({"accession": "E-MTAB-1", "max_rows_returned": 10}),
        )
        .await
        .unwrap();
    let studies = bio
        .call(
            "metabolights_get_studies",
            &json!({"accessions": ["MTBLS1", "MTBLS9"], "include_samples": true, "max_sample_rows_returned": 2}),
        )
        .await
        .unwrap();
    let mtbls_files = bio
        .call(
            "metabolights_get_study_files",
            &json!({"accession": "MTBLS1"}),
        )
        .await
        .unwrap();
    let data_files = bio
        .call(
            "metabolights_search_data_files",
            &json!({"accession": "MTBLS1", "pattern": "*.mzML"}),
        )
        .await
        .unwrap();
    let mgnify = bio
        .call(
            "mgnify_get_studies",
            &json!({"accessions": ["MGYS00000001", "MGYS00000009"], "include_analyses": true}),
        )
        .await
        .unwrap();
    let analyses = bio
        .call(
            "mgnify_get_study_analyses",
            &json!({"accession": "MGYS00000001"}),
        )
        .await
        .unwrap();
    let biome = bio
        .call(
            "mgnify_search_studies",
            &json!({"biome_lineage": "root:Host-associated:Human"}),
        )
        .await
        .unwrap();
    let projects = bio
        .call(
            "pride_get_projects",
            &json!({"accessions": ["PXD000001", "PXD000009"]}),
        )
        .await
        .unwrap();
    let proteins = bio
        .call(
            "pride_search_project_proteins",
            &json!({"project_accession": "PAD000001"}),
        )
        .await
        .unwrap();
    let protein_projects = bio
        .call(
            "pride_find_projects_for_protein",
            &json!({"protein_accession": "P04637"}),
        )
        .await
        .unwrap();
    server.abort();

    assert_eq!(geo["missing"], json!(["GSE9"]));
    assert_eq!(
        geo["records"][0]["samples"][0]["characteristics"][0]["tag"],
        "tissue"
    );
    assert_eq!(
        experiment["url"],
        "https://www.ebi.ac.uk/biostudies/arrayexpress/studies/E-MTAB-1"
    );
    assert_eq!(files["n_files"], 1);
    assert!(files["files"][0]["download_url"]
        .as_str()
        .unwrap()
        .starts_with("https://www.ebi.ac.uk/biostudies/files/E-MTAB-1/"));
    assert_eq!(samples["n_samples"], 1);
    assert_eq!(samples["samples"][0]["Source Name"], "SAMPLE-1");
    assert_eq!(studies["not_found"], json!(["MTBLS9"]));
    assert_eq!(
        studies["records"][0]["sample_table"]["rows_truncated"],
        true
    );
    assert_eq!(
        studies["records"][0]["url"],
        "https://www.ebi.ac.uk/metabolights/MTBLS1"
    );
    assert_eq!(mtbls_files["n_data_files"], 2);
    assert_eq!(data_files["files"], json!(["FILES/run1.mzML"]));
    assert_eq!(mgnify["missing"], json!(["MGYS00000009"]));
    assert_eq!(
        mgnify["studies"][0]["analyses"][0]["analysis_accession"],
        "MGYA00000001"
    );
    assert_eq!(analyses["analyses_count"], 1);
    assert_eq!(biome["returned"], 1);
    assert_eq!(projects["not_found"], json!(["PXD000009"]));
    assert_eq!(
        projects["records"][0]["url"],
        "https://www.ebi.ac.uk/pride/archive/projects/PXD000001"
    );
    assert_eq!(proteins["proteins"][0]["gene"], "TP53");
    assert_eq!(
        protein_projects["records"][0]["project_urls"][0],
        "https://www.ebi.ac.uk/pride/archive/projects/PXD000001"
    );
}

#[tokio::test]
async fn encodes_ncbi_identity_and_rejects_mgnify_ambiguous_search() {
    let captured = Arc::new(StdMutex::new(String::new()));
    let body = captured.clone();
    let app = Router::new().route(
        "/entrez/eutils/esearch.fcgi",
        post(move |incoming: String| {
            let body = body.clone();
            async move {
                *body.lock().unwrap() = incoming;
                Json(json!({"esearchresult": {"count": "0", "idlist": []}}))
            }
        }),
    );
    let (bio, server) = serve(app).await;
    let result = bio
        .call("geo_search_series", &json!({"term": "GSE1[ACCN] & liver"}))
        .await
        .unwrap();
    let error = bio
        .call(
            "mgnify_search_studies",
            &json!({"query": "gut", "biome_lineage": "root:Host-associated:Human"}),
        )
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    let form = captured.lock().unwrap().clone();
    assert!(form.contains("term=GSE1%5BACCN%5D+%26+liver"), "{form}");
    assert!(form.contains("api_key=synthetic-key%26value"), "{form}");
    assert!(form.contains("email=operator%40example.test"), "{form}");
    assert!(form.contains("tool=wisp-science"), "{form}");
    assert_eq!(result["returned"], 0);
    assert!(!result.to_string().contains("synthetic-key"));
    assert!(error.contains("exactly one"), "{error}");
}

#[tokio::test]
async fn rejects_rate_limits_malformed_json_and_oversized_bodies() {
    for (status, body, expected) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            "synthetic-key".into(),
            "HTTP 429",
        ),
        (StatusCode::OK, "{not-json".into(), "invalid JSON"),
        (
            StatusCode::OK,
            json!({"error": "invalid api_key synthetic-key"}).to_string(),
            "rejected",
        ),
        (
            StatusCode::OK,
            " ".repeat(MAX_RESPONSE + 1),
            "exceeded 4 MiB",
        ),
    ] {
        let app = Router::new().route(
            "/entrez/eutils/esearch.fcgi",
            post({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, [("retry-after", "60")], body).into_response() }
                }
            }),
        );
        let (bio, server) = serve(app).await;
        let error = bio
            .call("geo_search_series", &json!({"term": "synthetic"}))
            .await
            .unwrap_err()
            .to_string();
        server.abort();
        assert!(
            error.contains(expected),
            "{error} did not contain {expected}"
        );
        assert!(!error.contains("synthetic-key"), "{error}");
    }
}

#[tokio::test]
async fn unknown_tool_and_empty_sdrf_are_explicit() {
    let app = Router::new().route(
        "/biostudies/api/v1/studies/{acc}",
        get(|| async {
            Json(json!({
                "accno": "E-MTAB-8",
                "section": {"type": "Study", "attributes": [{"name": "Title", "value": "No SDRF"}]}
            }))
        }),
    );
    let (bio, server) = serve(app).await;
    let missing = bio
        .call(
            "arrayexpress_get_experiment_samples",
            &json!({"accession": "E-MTAB-8"}),
        )
        .await
        .unwrap();
    let unknown = bio
        .call("not_an_omics_tool", &json!({}))
        .await
        .unwrap_err()
        .to_string();
    server.abort();
    assert_eq!(missing["error"], "no_sdrf");
    assert!(
        unknown.contains("unknown native biological tool"),
        "{unknown}"
    );
}
