use assert_cmd::prelude::*;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    Command::cargo_bin("finance-cli").expect("finance-cli binary")
}

fn envs<'a>(cmd: &'a mut Command, config_dir: &Path, data_dir: &Path) -> &'a mut Command {
    cmd.env("FINANCE_OS_CONFIG_DIR", config_dir)
        .env("FINANCE_OS_DATA_DIR", data_dir)
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn seed_fixture_sync(temp: &TempDir, config_dir: &Path, data_dir: &Path) {
    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "shared_credit", "pluggyAccountId": "fixture-credit" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,fixture-credit,item-2,3,10\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31"),
        config_dir,
        data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("- transactions: 2"));
}

#[test]
fn milestone_zero_local_sync_and_report() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("local-dev"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("report")
            .arg("daily-pulse")
            .arg("--days")
            .arg("30"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Supermercado Angeloni"))
    .stdout(predicate::str::contains("Pagamento recebido"));
}

#[test]
fn import_legacy_is_idempotent() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    let finance_root = temp.path().join("legacy");
    fs::create_dir_all(finance_root.join("data/2026")).expect("create legacy data dir");

    write_file(
        &finance_root.join("data/contas.csv"),
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\n",
    );
    write_file(
        &finance_root.join("data/2026/transacoes.csv"),
        "pluggy_id,data,mes_ref,mes_movimento_ref,fatura_mes_ref,data_fechamento_fatura,data_vencimento,data_pagamento,status_pagamento,competencia_tipo,conta_id,conta_owner,conta_tipo,conta_banco,conta_label,valor,descricao,descricao_raw,categoria,subcategoria,descricao_canonica,contexto_finalidade,recorrencia_tipo,recorrencia_chave,recorrencia_frequencia,recorrencia_dia,recorrencia_origem,classificacao_fonte,classificacao_regra,tipo,data_hora_iso,pluggy_account_id,pluggy_amount_raw,pluggy_currency_code,pluggy_amount_in_account_currency,pluggy_balance,pluggy_category,pluggy_category_id,pluggy_provider_code,pluggy_provider_id,pluggy_status,pluggy_operation_type,pluggy_order,pluggy_created_at,pluggy_updated_at,payment_data_json,payment_method,payment_reason,payment_reference_number,payment_receiver_reference_id,payment_boleto_digitable_line,payment_boleto_barcode,payment_boleto_base_amount,payment_boleto_interest_amount,payment_boleto_penalty_amount,payment_boleto_discount_amount,payer_name,payer_branch_number,payer_account_number,payer_routing_number,payer_routing_number_ispb,payer_document_type,payer_document_value,receiver_name,receiver_branch_number,receiver_account_number,receiver_routing_number,receiver_routing_number_ispb,receiver_document_type,receiver_document_value,credit_card_metadata_json,credit_card_card_number,credit_card_payee_mcc,credit_card_installment_number,credit_card_total_installments,credit_card_total_amount,credit_card_purchase_date,credit_card_bill_id,merchant_json,merchant_name,merchant_business_name,merchant_cnpj,merchant_cnae,merchant_category,acquirer_data_json,raw_transaction_json\npluggy-1,2026-03-01,2026-03,2026-03,,,,,pago,conta_corrente,primary_checking,primary,checking,fintech,Primary Checking,-42.50,Mercado,Mercado,Alimentacao,Mercado,,, ,,,, ,rule,mercado_rule,DEBIT,2026-03-01T12:00:00.000Z,fixture-checking,,,,,,,,,POSTED,,,,2026-03-01T12:00:00.000Z,2026-03-01T12:00:00.000Z,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n",
    );
    write_file(
        &finance_root.join("contexto_transacoes.csv"),
        "match_type,match_value,valor_match,categoria,subcategoria,rotulo,finalidade,tipo_contexto,notas,ativo,revisado_em\ndescricao_contains,mercado,,Alimentacao,Mercado,Mercado do mês,Compras de mercado,contexto,,1,2026-03-01\n",
    );
    write_file(
        &finance_root.join("data/forecast_templates.csv"),
        "id,tipo,descricao,categoria,subcategoria,conta_id,valor,frequencia,dia_vencimento,inicio_mes,fim_mes,parcelas_total,match_contains,status,impacta_total,origem,notas\naluguel,recorrente,Aluguel,Moradia,Aluguel,primary_checking,1000.00,mensal,5,2026-03,2026-12,,aluguel,ativo,true,teste,\n",
    );
    write_file(
        &finance_root.join("rules.yaml"),
        "version: 1\ncategories:\n  - id: mercado_rule\n    match:\n      contains_any: [\"mercado\"]\n    set:\n      category: Alimentacao\n      subcategory: Mercado\n",
    );

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    for _ in 0..2 {
        envs(
            cargo_bin()
                .arg("admin")
                .arg("import-legacy")
                .arg("--finance-root")
                .arg(&finance_root),
            &config_dir,
            &data_dir,
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("- transactions: 1"));
    }

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let tx_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM transactions", [], |row| row.get(0))
        .expect("count transactions");
    let rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM rules", [], |row| row.get(0))
        .expect("count rules");
    let forecast_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM forecast", [], |row| row.get(0))
        .expect("count forecast");

    assert_eq!(tx_count, 1);
    assert_eq!(rule_count, 2);
    assert_eq!(forecast_count, 1);
}

#[test]
fn mutating_commands_feed_reporting_views() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("account")
            .arg("upsert")
            .arg("--account-id")
            .arg("reserva_emergencia")
            .arg("--owner")
            .arg("primary")
            .arg("--account-type")
            .arg("savings")
            .arg("--bank")
            .arg("itau")
            .arg("--label")
            .arg("Reserva Emergencia"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Conta salva: reserva_emergencia"));

    envs(
        cargo_bin()
            .arg("rule")
            .arg("upsert")
            .arg("--rule-id")
            .arg("farmacia_rule")
            .arg("--body")
            .arg("if description contains farmacia then category saude:farmacia"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Rule salva: farmacia_rule"));

    envs(
        cargo_bin()
            .arg("forecast")
            .arg("upsert")
            .arg("--forecast-id")
            .arg("mercado_planejado")
            .arg("--date")
            .arg("2026-03-30")
            .arg("--description")
            .arg("Mercado planejado")
            .arg("--amount")
            .arg("200.00")
            .arg("--category")
            .arg("Alimentacao")
            .arg("--subcategory")
            .arg("Mercado")
            .arg("--account-id")
            .arg("primary_checking"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Forecast salvo: mercado_planejado",
    ));

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual-card-001")
            .arg("--account-id")
            .arg("shared_credit")
            .arg("--date")
            .arg("2026-03-27")
            .arg("--description")
            .arg("Farmacia Cartao")
            .arg("--amount=-87.45")
            .arg("--payment-status")
            .arg("pending"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Transação manual salva: manual-card-001",
    ));

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual-checking-001")
            .arg("--account-id")
            .arg("primary_checking")
            .arg("--date")
            .arg("2026-03-27")
            .arg("--description")
            .arg("Exame Laboratorio")
            .arg("--amount=-120.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Transação manual salva: manual-checking-001",
    ));

    envs(
        cargo_bin()
            .arg("tx")
            .arg("categorize")
            .arg("--transaction-id")
            .arg("manual-checking-001")
            .arg("--category")
            .arg("Saude")
            .arg("--subcategory")
            .arg("Exames")
            .arg("--context")
            .arg("Exame de rotina"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context")
            .arg("--transaction-id")
            .arg("manual-checking-001")
            .arg("--context")
            .arg("Exame de rotina anual"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("report")
            .arg("monthly-spend")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("alimentacao-mercado"))
    .stdout(predicate::str::contains("saude:exames"))
    .stdout(predicate::str::contains("financeiro-pagamento-recebido").not());

    envs(
        cargo_bin()
            .arg("report")
            .arg("cashflow")
            .arg("--months")
            .arg("1"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("2026-03"))
    .stdout(predicate::str::contains("líquido"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("forecast-vs-actual")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Mercado planejado"))
    .stdout(predicate::str::contains("realizado"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("card-summary")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("shared_credit"))
    .stdout(predicate::str::contains("em aberto"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("uncategorized")
            .arg("--limit")
            .arg("10"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Farmacia Cartao"))
    .stdout(predicate::str::contains("Exame Laboratorio").not());

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let saved_context: String = conn
        .query_row(
            "SELECT context FROM transactions WHERE transaction_id = 'manual-checking-001'",
            [],
            |row| row.get(0),
        )
        .expect("saved context");
    let saved_category: String = conn
        .query_row(
            "SELECT category_id FROM transactions WHERE transaction_id = 'manual-checking-001'",
            [],
            |row| row.get(0),
        )
        .expect("saved category");
    let account_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM accounts WHERE account_id = 'reserva_emergencia'",
            [],
            |row| row.get(0),
        )
        .expect("count account");
    let rule_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rules WHERE rule_id = 'farmacia_rule'",
            [],
            |row| row.get(0),
        )
        .expect("count rule");
    let forecast_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM forecast WHERE forecast_id = 'mercado_planejado'",
            [],
            |row| row.get(0),
        )
        .expect("count forecast");

    assert_eq!(saved_context, "Exame de rotina anual");
    assert_eq!(saved_category, "saude:exames");
    assert_eq!(account_count, 1);
    assert_eq!(rule_count, 1);
    assert_eq!(forecast_count, 1);
}

#[test]
fn sync_json_summary_counts_only_new_transactions() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "shared_credit", "pluggyAccountId": "fixture-credit" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,fixture-credit,item-2,3,10\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    let first = envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--json-summary"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("first sync output");
    assert!(first.status.success());
    let first_json: Value = serde_json::from_slice(&first.stdout).expect("first sync json");
    assert_eq!(first_json["newTransactionsCount"], 2);
    assert_eq!(first_json["needsContextCount"], 0);
    assert_eq!(first_json["newTransactions"].as_array().unwrap().len(), 2);

    let second = envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--json-summary"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("second sync output");
    assert!(second.status.success());
    let second_json: Value = serde_json::from_slice(&second.stdout).expect("second sync json");
    assert_eq!(second_json["newTransactionsCount"], 0);
    assert_eq!(second_json["needsContextCount"], 0);
    assert_eq!(second_json["newTransactions"].as_array().unwrap().len(), 0);
}
