//! The three flows end to end on the in-memory adapters, with fakes for the externals.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use orderly::adapters::memory::{InMemoryOrderRepository, InMemoryOutbox};
use orderly::app::place::PlaceOrderCommand;
use orderly::app::ship::ShipOrderCommand;
use orderly::app::Services;
use orderly::domain::{Address, Carrier, Currency, LineItem, Money, Notification, OrderId, OrderStatus, TrackingNumber};
use orderly::ports::{GatewayError, Notifier, NotifyError, OrderRepository, PaymentGateway, ProviderRef, ShippingError, ShippingProvider};

struct FakeGateway {
    decline: bool,
}

#[async_trait]
impl PaymentGateway for FakeGateway {
    async fn charge(&self, order: OrderId, _amount: Money) -> Result<ProviderRef, GatewayError> {
        if self.decline {
            Err(GatewayError::Declined("insufficient funds".into()))
        } else {
            Ok(ProviderRef(format!("ch_{order}")))
        }
    }
    async fn refund(&self, _r: &ProviderRef) -> Result<(), GatewayError> {
        Ok(())
    }
}

struct FakeCarrier;

#[async_trait]
impl ShippingProvider for FakeCarrier {
    async fn book(&self, order: OrderId, carrier: Carrier, _to: &Address) -> Result<TrackingNumber, ShippingError> {
        Ok(TrackingNumber(format!("{}-{order}", carrier.code())))
    }
}

#[derive(Default)]
struct RecordingNotifier {
    sent: Mutex<Vec<Notification>>,
    fail_first: AtomicUsize,
}

#[async_trait]
impl Notifier for RecordingNotifier {
    async fn send(&self, n: &Notification) -> Result<(), NotifyError> {
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(NotifyError::Unreachable("down".into()));
        }
        self.sent.lock().unwrap().push(n.clone());
        Ok(())
    }
}

struct Harness {
    services: Services,
    repo: Arc<InMemoryOrderRepository>,
    outbox: Arc<InMemoryOutbox>,
    notifier: Arc<RecordingNotifier>,
}

fn harness(decline: bool, fail_first: usize) -> Harness {
    let repo = Arc::new(InMemoryOrderRepository::new());
    let outbox = Arc::new(InMemoryOutbox::new());
    let notifier = Arc::new(RecordingNotifier { fail_first: AtomicUsize::new(fail_first), ..Default::default() });
    let services = Services::new(repo.clone(), Arc::new(FakeGateway { decline }), Arc::new(FakeCarrier), notifier.clone(), outbox.clone());
    Harness { services, repo, outbox, notifier }
}

fn line() -> LineItem {
    LineItem { sku: "mug".into(), quantity: 2, unit_price: Money::new(1250, Currency::Eur) }
}

fn address() -> Address {
    Address { line1: "1 rue de la Paix".into(), city: "Paris".into(), postcode: "75002".into(), country: "FR".into() }
}

#[tokio::test]
async fn pay_then_ship_then_notify() {
    let h = harness(false, 0);
    let order = h.services.place.run(PlaceOrderCommand { customer_email: "a@b.c".into(), lines: vec![line()] }).await.unwrap();

    let payment = h.services.pay.run(order.id).await.unwrap();
    assert!(payment.is_captured());
    assert_eq!(h.repo.get(order.id).await.unwrap().status, OrderStatus::Paid);

    let shipment = h.services.ship.run(ShipOrderCommand { order: order.id, carrier: None, to: address() }).await.unwrap();
    assert_eq!(shipment.carrier, Carrier::Local);
    assert_eq!(h.repo.get(order.id).await.unwrap().status, OrderStatus::Shipped);
    assert_eq!(h.outbox.pending(), 2);

    let report = h.services.notify.drain(10).await.unwrap();
    assert_eq!(report.sent, 2);
    assert_eq!(h.outbox.pending(), 0);
    let sent = h.notifier.sent.lock().unwrap();
    assert!(sent[0].subject.contains("confirmed"));
    assert!(sent[1].body.contains("local-"));
}

#[tokio::test]
async fn declined_payment_records_failure_and_leaves_order_placed() {
    let h = harness(true, 0);
    let order = h.services.place.run(PlaceOrderCommand { customer_email: "a@b.c".into(), lines: vec![line()] }).await.unwrap();
    assert!(h.services.pay.run(order.id).await.is_err());
    assert_eq!(h.repo.get(order.id).await.unwrap().status, OrderStatus::Placed);
    assert_eq!(h.repo.payments().len(), 1);
    assert_eq!(h.outbox.pending(), 0);
}

#[tokio::test]
async fn unpaid_order_cannot_ship() {
    let h = harness(false, 0);
    let order = h.services.place.run(PlaceOrderCommand { customer_email: "a@b.c".into(), lines: vec![line()] }).await.unwrap();
    assert!(h.services.ship.run(ShipOrderCommand { order: order.id, carrier: Some(Carrier::Ups), to: address() }).await.is_err());
}

#[tokio::test]
async fn notify_retries_released_messages() {
    let h = harness(false, 1);
    let order = h.services.place.run(PlaceOrderCommand { customer_email: "a@b.c".into(), lines: vec![line()] }).await.unwrap();
    h.services.pay.run(order.id).await.unwrap();
    let first = h.services.notify.drain(10).await.unwrap();
    assert_eq!((first.sent, first.retried), (0, 1));
    let second = h.services.notify.drain(10).await.unwrap();
    assert_eq!((second.sent, second.retried), (1, 0));
}
