package io.atlas.payments

import atlas.events.FareEvent
import io.atlas.payments.core.FakePaymentProvider
import io.atlas.payments.core.PaymentError
import io.atlas.payments.core.PaymentsService
import io.atlas.payments.core.TxStatus
import io.atlas.payments.fakes.DirectTransactionRunner
import io.atlas.payments.fakes.InMemoryOutbox
import io.atlas.payments.fakes.InMemoryTransactionRepository
import io.atlas.payments.fakes.InMemoryWalletRepository
import java.util.UUID
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class PaymentsServiceTest {
    private val wallets = InMemoryWalletRepository()
    private val transactions = InMemoryTransactionRepository()
    private val outbox = InMemoryOutbox()
    private val topic = "atlas.fare.events"

    private val service = PaymentsService(
        wallets = wallets,
        transactions = transactions,
        outbox = outbox,
        runner = DirectTransactionRunner(),
        provider = FakePaymentProvider(),
        fareTopic = topic,
    )

    private val rider = UUID.randomUUID().toString()
    private val driver = UUID.randomUUID().toString()
    private val ride = UUID.randomUUID().toString()

    private fun lastEventType(): FareEvent.EventType =
        FareEvent.parseFrom(outbox.all().last().payload).eventType

    // --- initiate ---------------------------------------------------------

    @Test
    fun `initiate creates a pending transaction and a RIDE_ACCEPTED outbox event`() {
        val result = service.initiate(rider, driver, 1500, "key-1", ride)

        assertEquals(TxStatus.PENDING, result.status)
        val tx = transactions.findById(result.transactionId)!!
        assertEquals(1500, tx.amountCents)
        assertEquals(TxStatus.PENDING, tx.status)
        assertTrue(tx.providerRef!!.startsWith("fake_"))
        assertEquals(1, outbox.all().size)
        assertEquals(FareEvent.EventType.RIDE_ACCEPTED, lastEventType())
    }

    @Test
    fun `initiate is idempotent for the same key and args`() {
        val first = service.initiate(rider, driver, 1500, "key-1", ride)
        val second = service.initiate(rider, driver, 1500, "key-1", ride)

        assertEquals(first.transactionId, second.transactionId)
        assertEquals(1, outbox.all().size) // replay does not enqueue again
    }

    @Test
    fun `initiate rejects a reused key with different args`() {
        service.initiate(rider, driver, 1500, "key-1", ride)
        assertFailsWith<PaymentError.IdempotencyConflict> {
            service.initiate(rider, driver, 9999, "key-1", ride)
        }
    }

    @Test
    fun `initiate rejects a non-positive amount`() {
        assertFailsWith<PaymentError.InvalidAmount> {
            service.initiate(rider, driver, 0, "key-1", ride)
        }
    }

    @Test
    fun `initiate rejects identical payer and payee`() {
        assertFailsWith<PaymentError.InvalidArgument> {
            service.initiate(rider, rider, 1500, "key-1", ride)
        }
    }

    @Test
    fun `initiate rejects a non-uuid user id`() {
        assertFailsWith<PaymentError.InvalidArgument> {
            service.initiate("not-a-uuid", driver, 1500, "key-1", ride)
        }
    }

    // --- settle -----------------------------------------------------------

    @Test
    fun `settle moves balances and emits TRANSACTION_SETTLED`() {
        val tx = transactions.findById(service.initiate(rider, driver, 1500, "key-1", ride).transactionId)!!
        // Fund the payer so the capture has something to move.
        wallets.adjustBalance(tx.fromWallet!!, 5000)

        val result = service.settle(tx.id.toString())

        assertTrue(result.success)
        assertEquals(TxStatus.SETTLED, result.status)
        assertEquals(3500, wallets.findById(tx.fromWallet!!)!!.balanceCents)
        assertEquals(1500, wallets.findById(tx.toWallet!!)!!.balanceCents)
        assertEquals(FareEvent.EventType.TRANSACTION_SETTLED, lastEventType())
    }

    @Test
    fun `settle is idempotent`() {
        val tx = transactions.findById(service.initiate(rider, driver, 1500, "key-1", ride).transactionId)!!
        wallets.adjustBalance(tx.fromWallet!!, 5000)
        service.settle(tx.id.toString())
        val outboxAfterFirst = outbox.all().size

        val again = service.settle(tx.id.toString())

        assertTrue(again.success)
        assertEquals(outboxAfterFirst, outbox.all().size) // no second event
    }

    @Test
    fun `settle rejects insufficient funds`() {
        val tx = transactions.findById(service.initiate(rider, driver, 1500, "key-1", ride).transactionId)!!
        // Payer wallet left at zero balance.
        assertFailsWith<PaymentError.InsufficientFunds> {
            service.settle(tx.id.toString())
        }
    }

    @Test
    fun `settle of an unknown transaction is NOT_FOUND`() {
        assertFailsWith<PaymentError.TransactionNotFound> {
            service.settle(UUID.randomUUID().toString())
        }
    }

    // --- refund -----------------------------------------------------------

    @Test
    fun `refund reverses a settled transaction and emits TRANSACTION_REFUNDED`() {
        val tx = transactions.findById(service.initiate(rider, driver, 1500, "key-1", ride).transactionId)!!
        wallets.adjustBalance(tx.fromWallet!!, 5000)
        service.settle(tx.id.toString())

        val result = service.refund(tx.id.toString())

        assertTrue(result.success)
        assertEquals(TxStatus.REFUNDED, transactions.findById(tx.id)!!.status)
        assertEquals(5000, wallets.findById(tx.fromWallet!!)!!.balanceCents) // refunded back
        assertEquals(0, wallets.findById(tx.toWallet!!)!!.balanceCents)
        assertEquals(FareEvent.EventType.TRANSACTION_REFUNDED, lastEventType())
    }

    @Test
    fun `refund of a pending transaction is rejected`() {
        val tx = transactions.findById(service.initiate(rider, driver, 1500, "key-1", ride).transactionId)!!
        assertFailsWith<PaymentError.InvalidState> {
            service.refund(tx.id.toString())
        }
    }

    // --- wallet -----------------------------------------------------------

    @Test
    fun `walletBalance returns zero for an unknown user`() {
        val balance = service.walletBalance(UUID.randomUUID().toString())
        assertEquals(0, balance.balanceCents)
        assertEquals("USD", balance.currency)
    }

    @Test
    fun `walletBalance reflects a credited wallet`() {
        val tx = transactions.findById(service.initiate(rider, driver, 1500, "key-1", ride).transactionId)!!
        wallets.adjustBalance(tx.fromWallet!!, 5000)
        val balance = service.walletBalance(rider)
        assertEquals(5000, balance.balanceCents)
    }

    @AfterTest
    fun noOp() = Unit
}
