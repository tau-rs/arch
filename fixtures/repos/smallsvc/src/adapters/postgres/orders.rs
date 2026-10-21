use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::{Currency, LineItem, Money, Order, OrderId, OrderStatus, Payment, Shipment};
use crate::ports::{OrderRepository, RepoError};

pub struct PgOrderRepository {
    pool: PgPool,
}

impl PgOrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct OrderRow {
    id: Uuid,
    customer_email: String,
    status: String,
    placed_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct LineRow {
    sku: String,
    quantity: i32,
    unit_price_cents: i64,
    currency: String,
}

fn currency_from(code: &str) -> Result<Currency, RepoError> {
    match code {
        "EUR" => Ok(Currency::Eur),
        "USD" => Ok(Currency::Usd),
        other => Err(RepoError::Corrupt(format!("currency {other}"))),
    }
}

fn to_order(row: OrderRow, lines: Vec<LineRow>) -> Result<Order, RepoError> {
    let status = OrderStatus::parse(&row.status).ok_or_else(|| RepoError::Corrupt(format!("status {}", row.status)))?;
    let lines = lines
        .into_iter()
        .map(|l| {
            Ok(LineItem {
                sku: l.sku,
                quantity: l.quantity.max(0) as u32,
                unit_price: Money::new(l.unit_price_cents, currency_from(&l.currency)?),
            })
        })
        .collect::<Result<Vec<_>, RepoError>>()?;
    Ok(Order { id: OrderId(row.id), customer_email: row.customer_email, lines, status, placed_at: row.placed_at })
}

fn storage(e: sqlx::Error) -> RepoError {
    RepoError::Storage(e.to_string())
}

#[async_trait]
impl OrderRepository for PgOrderRepository {
    async fn get(&self, id: OrderId) -> Result<Order, RepoError> {
        let row: Option<OrderRow> = sqlx::query_as("SELECT id, customer_email, status, placed_at FROM orders WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        let row = row.ok_or(RepoError::NotFound(id))?;
        let lines: Vec<LineRow> = sqlx::query_as("SELECT sku, quantity, unit_price_cents, currency FROM order_lines WHERE order_id = $1 ORDER BY position")
            .bind(id.0)
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        to_order(row, lines)
    }

    async fn save(&self, order: &Order) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query(
            "INSERT INTO orders (id, customer_email, status, placed_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status",
        )
        .bind(order.id.0)
        .bind(&order.customer_email)
        .bind(order.status.as_str())
        .bind(order.placed_at)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        sqlx::query("DELETE FROM order_lines WHERE order_id = $1").bind(order.id.0).execute(&mut *tx).await.map_err(storage)?;
        for (position, line) in order.lines.iter().enumerate() {
            sqlx::query("INSERT INTO order_lines (order_id, position, sku, quantity, unit_price_cents, currency) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(order.id.0)
                .bind(position as i32)
                .bind(&line.sku)
                .bind(line.quantity as i32)
                .bind(line.unit_price.cents)
                .bind(line.unit_price.currency.code())
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        tx.commit().await.map_err(storage)
    }

    async fn list_by_status(&self, status: OrderStatus) -> Result<Vec<Order>, RepoError> {
        let rows: Vec<OrderRow> = sqlx::query_as("SELECT id, customer_email, status, placed_at FROM orders WHERE status = $1 ORDER BY placed_at")
            .bind(status.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id = OrderId(row.id);
            out.push(self.get(id).await?);
        }
        Ok(out)
    }

    async fn record_payment(&self, payment: &Payment) -> Result<(), RepoError> {
        sqlx::query("INSERT INTO payments (id, order_id, amount_cents, currency, status, provider_ref, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(payment.id.0)
            .bind(payment.order.0)
            .bind(payment.amount.cents)
            .bind(payment.amount.currency.code())
            .bind(format!("{:?}", payment.status).to_lowercase())
            .bind(&payment.provider_ref)
            .bind(payment.created_at)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn record_shipment(&self, shipment: &Shipment) -> Result<(), RepoError> {
        sqlx::query("INSERT INTO shipments (id, order_id, carrier, tracking, status, booked_at) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(shipment.id.0)
            .bind(shipment.order.0)
            .bind(shipment.carrier.code())
            .bind(&shipment.tracking.0)
            .bind(format!("{:?}", shipment.status).to_lowercase())
            .bind(shipment.booked_at)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}
