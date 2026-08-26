package io.atlas.auth.core

import org.slf4j.LoggerFactory

/**
 * Outbound email.
 *
 * Atlas does not send mail itself and deliberately does not try to. Real
 * delivery is a provider integration — SES, Postmark, Resend — with its
 * own credentials, bounce handling, and reputation management, and the
 * shape of that work has nothing to do with the auth logic that needs it.
 * So this is a port, exactly like [io.atlas.payments.core.PaymentProvider]
 * on the money side: the logic is written against the interface, and the
 * provider is a deployment decision.
 *
 * # What the implementations mean
 *
 * [LoggingEmailSender] writes the message to the log and returns success.
 * That is the right behaviour for local development — a developer testing
 * a reset flow needs to see the link, and standing up a mail provider to
 * do it would be absurd — and it is the wrong behaviour in production,
 * where it would print reset links into the log aggregator for anyone
 * with log access to redeem.
 *
 * The service refuses to start with this implementation unless
 * ATLAS_ALLOW_LOGGING_EMAIL is set, so shipping it is a decision somebody
 * has to make on purpose rather than a default nobody noticed.
 */
interface EmailSender {
    /**
     * Deliver one message. Implementations must not throw for an
     * undeliverable address: a bounce is normal and is not the caller's
     * error to handle synchronously. Throw only for faults the caller
     * could retry through, such as the provider being unreachable.
     */
    fun send(message: EmailMessage)
}

data class EmailMessage(
    val to: String,
    val subject: String,
    val body: String,
)

/**
 * Development sender. Logs the message and drops it.
 *
 * The body is logged in full, including any token in it, which is the
 * entire point locally and precisely the danger in production.
 */
class LoggingEmailSender : EmailSender {
    override fun send(message: EmailMessage) {
        LOG.info(
            "EMAIL (not actually sent)\n  to: {}\n  subject: {}\n  body:\n{}",
            message.to,
            message.subject,
            message.body,
        )
    }

    private companion object {
        private val LOG = LoggerFactory.getLogger(LoggingEmailSender::class.java)
    }
}
