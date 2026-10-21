use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderId(pub Uuid);

impl OrderId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, OrderError> {
        Uuid::parse_str(s).map(Self).map_err(|_| OrderError::InvalidId(s.to_string()))
    }
}

impl Default for OrderId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OrderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Eur,
    Usd,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Currency::Eur => "EUR",
            Currency::Usd => "USD",
        }
    }
}

/// Minor units (cents) in one currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub cents: i64,
    pub currency: Currency,
}

impl Money {
    pub const fn zero(currency: Currency) -> Self {
        Self { cents: 0, currency }
    }

    pub fn new(cents: i64, currency: Currency) -> Self {
        Self { cents, currency }
    }

    pub fn plus(self, other: Money) -> Result<Money, OrderError> {
        if self.currency != other.currency {
            return Err(OrderError::CurrencyMismatch);
        }
        Ok(Money { cents: self.cents + other.cents, currency: self.currency })
    }

    pub fn times(self, qty: u32) -> Money {
        Money { cents: self.cents * i64::from(qty), currency: self.currency }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub sku: String,
    pub quantity: u32,
    pub unit_price: Money,
}

impl LineItem {
    pub fn total(&self) -> Money {
        self.unit_price.times(self.quantity)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Placed,
    Paid,
    Shipped,
    Delivered,
    Cancelled,
}

impl OrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OrderStatus::Placed => "placed",
            OrderStatus::Paid => "paid",
            OrderStatus::Shipped => "shipped",
            OrderStatus::Delivered => "delivered",
            OrderStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "placed" => OrderStatus::Placed,
            "paid" => OrderStatus::Paid,
            "shipped" => OrderStatus::Shipped,
            "delivered" => OrderStatus::Delivered,
            "cancelled" => OrderStatus::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub customer_email: String,
    pub lines: Vec<LineItem>,
    pub status: OrderStatus,
    pub placed_at: DateTime<Utc>,
}

impl Order {
    pub fn place(customer_email: String, lines: Vec<LineItem>) -> Result<Self, OrderError> {
        if lines.is_empty() {
            return Err(OrderError::Empty);
        }
        if !customer_email.contains('@') {
            return Err(OrderError::InvalidEmail(customer_email));
        }
        Ok(Self { id: OrderId::new(), customer_email, lines, status: OrderStatus::Placed, placed_at: Utc::now() })
    }

    pub fn total(&self) -> Result<Money, OrderError> {
        let currency = self.lines.first().map(|l| l.unit_price.currency).ok_or(OrderError::Empty)?;
        self.lines.iter().try_fold(Money::zero(currency), |acc, l| acc.plus(l.total()))
    }

    pub fn mark_paid(&mut self) -> Result<(), OrderError> {
        self.transition(OrderStatus::Placed, OrderStatus::Paid)
    }

    pub fn mark_shipped(&mut self) -> Result<(), OrderError> {
        self.transition(OrderStatus::Paid, OrderStatus::Shipped)
    }

    pub fn mark_delivered(&mut self) -> Result<(), OrderError> {
        self.transition(OrderStatus::Shipped, OrderStatus::Delivered)
    }

    pub fn cancel(&mut self) -> Result<(), OrderError> {
        match self.status {
            OrderStatus::Placed | OrderStatus::Paid => {
                self.status = OrderStatus::Cancelled;
                Ok(())
            }
            from => Err(OrderError::InvalidTransition { from, to: OrderStatus::Cancelled }),
        }
    }

    pub fn can_ship(&self) -> bool {
        self.status == OrderStatus::Paid
    }

    fn transition(&mut self, from: OrderStatus, to: OrderStatus) -> Result<(), OrderError> {
        if self.status != from {
            return Err(OrderError::InvalidTransition { from: self.status, to });
        }
        self.status = to;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OrderError {
    #[error("order {0} not found")]
    NotFound(OrderId),
    #[error("an order needs at least one line")]
    Empty,
    #[error("invalid order id {0}")]
    InvalidId(String),
    #[error("invalid customer email {0}")]
    InvalidEmail(String),
    #[error("cannot go from {from:?} to {to:?}")]
    InvalidTransition { from: OrderStatus, to: OrderStatus },
    #[error("lines mix currencies")]
    CurrencyMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(cents: i64, qty: u32) -> LineItem {
        LineItem { sku: "sku".into(), quantity: qty, unit_price: Money::new(cents, Currency::Eur) }
    }

    #[test]
    fn total_sums_lines() {
        let o = Order::place("a@b.c".into(), vec![line(100, 2), line(50, 1)]).unwrap();
        assert_eq!(o.total().unwrap(), Money::new(250, Currency::Eur));
    }

    #[test]
    fn lifecycle_is_linear() {
        let mut o = Order::place("a@b.c".into(), vec![line(1, 1)]).unwrap();
        assert!(o.mark_shipped().is_err());
        o.mark_paid().unwrap();
        assert!(o.can_ship());
        o.mark_shipped().unwrap();
        assert_eq!(o.cancel(), Err(OrderError::InvalidTransition { from: OrderStatus::Shipped, to: OrderStatus::Cancelled }));
    }
}
