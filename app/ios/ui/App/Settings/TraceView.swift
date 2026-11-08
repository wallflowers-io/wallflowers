//  TraceView.swift
//  The device half of the trace, on screen: what happened, newest first.
//
//  This view owns the stages no server can see (persistence, proposals, notification,
//  membership), and each row names the object it concerns.

import SwiftUI

struct TraceView: View {
    @ObservedObject private var trace = Trace.shared

    var body: some View {
        List {
            ForEach(trace.events) { e in
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(e.ok ? Color.green : Color.orange)
                            .frame(width: 7, height: 7)
                        Text(e.stage)
                            .font(.caption.weight(.semibold))
                        Spacer()
                        Text(e.at, style: .time)
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    Text(e.detail)
                        .font(.caption)
                        .fixedSize(horizontal: false, vertical: true)
                    if !e.object.isEmpty {
                        Text(e.object)
                            .font(.caption2.monospaced())
                            .foregroundStyle(.tertiary)
                            .textSelection(.enabled)
                    }
                }
                .padding(.vertical, 2)
            }
        }
        .navigationTitle("Trace")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Clear") { trace.clear() }
            }
        }
        .overlay {
            if trace.events.isEmpty {
                Text("Nothing traced yet")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }
}
