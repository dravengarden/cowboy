import AppKit
import CowboyInstallerCore
import SwiftUI

struct ActivityView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if model.isRunning {
                GroupBox("Current task") {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(model.installState.message)
                        ProgressView(value: model.installState.progress)
                        Text("\(Int(model.installState.progress * 100))%")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(8)
                }
            }

            if model.history.isEmpty {
                ContentUnavailableView(
                    "No Installation Activity",
                    systemImage: "clock.arrow.circlepath",
                    description: Text("Completed, failed, cancelled, and interrupted tasks appear here.")
                )
            } else {
                List(model.history) { record in
                    ActivityRow(record: record)
                }
            }
        }
        .padding(20)
        .navigationTitle("Activity")
    }
}

private struct ActivityRow: View {
    let record: ActivityRecord
    @State private var detailsExpanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack {
                Label(record.summary, systemImage: symbol)
                    .foregroundStyle(color)
                Spacer()
                Text(record.endedAt, style: .relative)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 16) {
                Text("Started \(record.startedAt.formatted(date: .abbreviated, time: .shortened))")
                Text("Finished \(record.endedAt.formatted(date: .abbreviated, time: .shortened))")
                Text("Version \(record.targetVersion)")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            DisclosureGroup("Details", isExpanded: $detailsExpanded) {
                Text(record.details)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 6)
                Button("Copy Details") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(record.details, forType: .string)
                }
                .accessibilityLabel("Copy activity details")
            }
        }
        .padding(.vertical, 6)
    }

    private var symbol: String {
        switch record.result {
        case .succeeded: "checkmark.circle.fill"
        case .failed: "exclamationmark.triangle.fill"
        case .cancelled: "xmark.circle"
        case .interrupted: "bolt.slash.circle"
        }
    }

    private var color: Color {
        switch record.result {
        case .succeeded: .green
        case .failed, .interrupted: .orange
        case .cancelled: .secondary
        }
    }
}
