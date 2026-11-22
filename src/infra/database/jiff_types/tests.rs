//! jiff 型別與 PostgreSQL 的往返。

use jiff::civil::date;
use jiff_sqlx::ToSqlx;

use super::*;
use crate::infra::database::get_connection;

#[derive(sqlx::FromRow)]
struct JiffRow {
    #[sqlx(try_from = "jiff_sqlx::Date")]
    day: Date,
    #[sqlx(try_from = "Nullable<jiff_sqlx::Date>")]
    missing_day: Option<Date>,
    #[sqlx(try_from = "Nullable<jiff_sqlx::Date>")]
    present_day: Option<Date>,
    #[sqlx(try_from = "jiff_sqlx::DateTime")]
    wall_clock: DateTime,
    #[sqlx(try_from = "Nullable<jiff_sqlx::Time>")]
    clock: Option<Time>,
    moment: TaipeiDateTime,
    missing_moment: Option<TaipeiDateTime>,
}

/// 綁定 jiff 值寫入、以 FromRow 讀回，並與直接讀 SQL 字面值的結果比對。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn jiff_values_round_trip() {
    dotenvy::dotenv().ok();
    let moment: TaipeiDateTime = "2026-11-22T13:05:09.123456+08:00".parse().expect("解析");
    let present: Option<Date> = Some(date(2024, 2, 29));
    let row: JiffRow = sqlx::query_as(
        "SELECT $1::date AS day, NULL::date AS missing_day, $2::date AS present_day, \
         $3::timestamp AS wall_clock, $4::time AS clock, $5::timestamptz AS moment, \
         NULL::timestamptz AS missing_moment",
    )
    .bind(date(2026, 11, 22).to_sqlx())
    .bind(present.map(ToSqlx::to_sqlx))
    .bind(date(2026, 11, 22).at(13, 5, 9, 0).to_sqlx())
    .bind(Time::new(9, 0, 0, 0).expect("合法").to_sqlx())
    .bind(moment)
    .fetch_one(get_connection())
    .await
    .expect("查詢");
    assert_eq!(row.day, date(2026, 11, 22));
    assert_eq!(row.missing_day, None);
    assert_eq!(row.present_day, Some(date(2024, 2, 29)));
    assert_eq!(row.wall_clock, date(2026, 11, 22).at(13, 5, 9, 0));
    assert_eq!(row.clock, Some(Time::new(9, 0, 0, 0).expect("合法")));
    assert_eq!(row.moment, moment);
    assert_eq!(row.missing_moment, None);

    // 直接讀 SQL 字面值：微秒精度的時間點與綁定寫入的相同
    let (literal_moment,): (TaipeiDateTime,) =
        sqlx::query_as("SELECT '2026-11-22T13:05:09.123456+08:00'::timestamptz")
            .fetch_one(get_connection())
            .await
            .expect("查詢");
    assert_eq!(literal_moment, moment);
    assert_eq!(
        literal_moment.timestamp().as_microsecond(),
        1_795_323_909_123_456
    );

    // 陣列綁定
    let days: Vec<jiff_sqlx::Date> = vec![date(2026, 1, 2).to_sqlx(), date(2026, 1, 5).to_sqlx()];
    let (count,): (i64,) = sqlx::query_as("SELECT cardinality($1::date[])::bigint")
        .bind(&days)
        .fetch_one(get_connection())
        .await
        .expect("查詢");
    assert_eq!(count, 2);
    let moments = vec![moment];
    let (first,): (TaipeiDateTime,) = sqlx::query_as("SELECT ($1::timestamptz[])[1]")
        .bind(&moments)
        .fetch_one(get_connection())
        .await
        .expect("查詢");
    assert_eq!(first, moment);
}

/// 非 NULL 的欄位讀到 NULL 時回傳解碼錯誤，不會變成預設值。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn null_into_required_date_is_an_error() {
    dotenvy::dotenv().ok();
    #[derive(sqlx::FromRow)]
    struct Required {
        #[sqlx(try_from = "jiff_sqlx::Date")]
        #[allow(dead_code)]
        day: Date,
    }
    let result = sqlx::query_as::<_, Required>("SELECT NULL::date AS day")
        .fetch_one(get_connection())
        .await;
    assert!(result.is_err(), "NULL 不能讀進非 Option 的日期");
}
