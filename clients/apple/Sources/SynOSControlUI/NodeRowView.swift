import SwiftUI
import SynOSClient

struct NodeRowView: View {
    let node: ClusterNode

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Node \(node.id)", systemImage: "server.rack")
                    .font(.headline)
                Spacer()
                Text(healthLabel)
                    .foregroundStyle(healthColor)
            }
            ProgressView("CPU", value: node.cpuFraction)
            ProgressView("Memory", value: node.memoryFraction)
            Text("\(node.runningJobs) running · \(node.queuedJobs) queued")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    private var healthLabel: String {
        switch node.health {
        case .healthy:
            "Healthy"
        case .degraded:
            "Degraded"
        case .failed:
            "Failed"
        }
    }

    private var healthColor: Color {
        switch node.health {
        case .healthy:
            .green
        case .degraded:
            .orange
        case .failed:
            .red
        }
    }
}
