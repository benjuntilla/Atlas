package io.atlas.fare.grpc

import atlas.payments.PaymentsServiceGrpcKt
import atlas.payments.RefundRequest
import atlas.payments.SettleRequest
import io.atlas.fare.core.CommandResult
import io.atlas.fare.core.PaymentsCommands
import io.grpc.ManagedChannel
import io.grpc.ManagedChannelBuilder
import io.grpc.Status
import io.grpc.StatusException
import kotlinx.coroutines.runBlocking
import org.slf4j.LoggerFactory
import java.util.UUID
import java.util.concurrent.TimeUnit

/**
 * [PaymentsCommands] over gRPC.
 *
 * The Kafka consumer loop is a plain blocking thread (the same shape as
 * auth-service's TokenEventConsumer), while the generated Kotlin stubs
 * are coroutine-based, so each call is bridged with `runBlocking`. That
 * is fine here and would not be in a server: this thread's entire job is
 * to process one event at a time in order, so there is nothing else it
 * could usefully be doing while the call is in flight.
 *
 * The status-code mapping is the interesting part. It decides whether a
 * failure advances the Kafka offset or replays the event, so getting it
 * wrong either loses settlements or spins forever on one poison message.
 */
class PaymentsGrpcCommands(
    private val channel: ManagedChannel,
    private val deadlineSeconds: Long = 10,
) : PaymentsCommands, AutoCloseable {

    private val stub = PaymentsServiceGrpcKt.PaymentsServiceCoroutineStub(channel)

    override fun settle(projectId: UUID, transactionId: UUID): CommandResult = call("settle") {
        stub.withDeadlineAfter(deadlineSeconds, TimeUnit.SECONDS)
            .settleTransaction(
                SettleRequest.newBuilder()
                    .setTransactionId(transactionId.toString())
                    // From the Kafka event, which is the only place a
                    // consumer can learn its tenant: the request that
                    // produced the event is long gone.
                    .setProjectId(projectId.toString())
                    .build(),
            )
    }

    override fun refund(projectId: UUID, transactionId: UUID): CommandResult = call("refund") {
        stub.withDeadlineAfter(deadlineSeconds, TimeUnit.SECONDS)
            .refundTransaction(
                RefundRequest.newBuilder()
                    .setTransactionId(transactionId.toString())
                    .setProjectId(projectId.toString())
                    .build(),
            )
    }

    private fun call(op: String, block: suspend () -> Any): CommandResult =
        try {
            runBlocking { block() }
            CommandResult.Applied
        } catch (e: StatusException) {
            classify(op, e)
        } catch (e: Exception) {
            // Anything not expressed as a gRPC status (channel shutdown,
            // interrupted thread) is treated as transient. Retrying a
            // settle is harmless; dropping one is not.
            CommandResult.Unavailable("$op failed: ${e.message}")
        }

    private fun classify(op: String, e: StatusException): CommandResult = when (e.status.code) {
        // The transaction is not in a state this operation accepts —
        // refunding a PENDING transaction, settling a REFUNDED one. The
        // world has to change before this could succeed, and redelivering
        // the same event will not change it.
        Status.Code.FAILED_PRECONDITION,
        Status.Code.NOT_FOUND,
        Status.Code.INVALID_ARGUMENT,
        -> CommandResult.Rejected("${e.status.code}: ${e.status.description}")

        // Payments is down, overloaded, or slow. Redelivery is exactly the
        // right response.
        Status.Code.UNAVAILABLE,
        Status.Code.DEADLINE_EXCEEDED,
        Status.Code.RESOURCE_EXHAUSTED,
        Status.Code.ABORTED,
        -> CommandResult.Unavailable("${e.status.code}: ${e.status.description}")

        // INTERNAL and friends are ambiguous. Treat them as retryable:
        // the money operations are idempotent, so a spurious retry costs
        // nothing, while giving up on a settle loses revenue silently.
        else -> {
            LOG.warn("unclassified status from {}: {}", op, e.status)
            CommandResult.Unavailable("${e.status.code}: ${e.status.description}")
        }
    }

    override fun close() {
        channel.shutdown().awaitTermination(5, TimeUnit.SECONDS)
    }

    companion object {
        private val LOG = LoggerFactory.getLogger(PaymentsGrpcCommands::class.java)

        fun build(target: String): PaymentsGrpcCommands {
            // Plaintext: this is an internal hop on a private network, the
            // same assumption the gateway makes for its upstreams.
            val channel = ManagedChannelBuilder.forTarget(target)
                .usePlaintext()
                .keepAliveTime(60, TimeUnit.SECONDS)
                .build()
            return PaymentsGrpcCommands(channel)
        }
    }
}
