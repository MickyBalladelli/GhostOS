import SwiftUI
import GhostOSClient

public struct OperatorNoticeView: View {
    private let failure: GhostOSOperatorFailure

    public init(failure: GhostOSOperatorFailure) {
        self.failure = failure
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label("Operation failed", systemImage: "exclamationmark.triangle.fill")
                .font(.headline)
            Text("Action: \(failure.action)")
            Text("Impact: \(failure.impact)")
            Text("Retry safety: \(failure.retrySafety)")
            Text("Audit: \(failure.auditCorrelation) · node \(failure.auditNode)")
                .font(.caption)
        }
        .foregroundStyle(.white)
        .padding(12)
        .background(.red.opacity(0.9), in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "Operation failed. Action: \(failure.action). Impact: \(failure.impact). Retry safety: \(failure.retrySafety). Audit identity: \(failure.auditCorrelation), node \(failure.auditNode)."
        )
    }
}
