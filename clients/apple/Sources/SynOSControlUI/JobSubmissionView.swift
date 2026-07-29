import SwiftUI

public struct JobSubmissionView: View {
    @ObservedObject private var store: ClusterStore
    @State private var command = ""
    @State private var priority = 10
    @State private var timeoutSeconds = 300

    public init(store: ClusterStore) {
        self.store = store
    }

    public var body: some View {
        Form {
            Section("Job") {
                TextField("Command", text: $command, axis: .vertical)
                    .lineLimit(3...8)
                Stepper("Priority \(priority)", value: $priority, in: 1...255)
                Stepper(
                    "Timeout \(timeoutSeconds) seconds",
                    value: $timeoutSeconds,
                    in: 1...86_400
                )
                Button("Submit") {
                    Task {
                        await store.submit(
                            command: command,
                            priority: UInt8(priority),
                            timeoutSeconds: UInt64(timeoutSeconds)
                        )
                    }
                }
                .disabled(command.isEmpty || store.isLoading)
            }
            if let receipt = store.lastJob {
                Section("Accepted") {
                    LabeledContent("Job ID", value: "\(receipt.jobID)")
                }
            }
        }
        .navigationTitle("Submit Job")
    }
}
