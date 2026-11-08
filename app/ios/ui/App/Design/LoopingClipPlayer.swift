//  LoopingClipPlayer.swift
//  A muted, looping, chrome-less clip.
//
//  It lived in EventDetailPage while the event page was the only surface with a
//  clip on it. That page has been rebuilt as a modal and the Space profile plays
//  clips too, so the atom moved to where both can see it rather than one page
//  importing another's internals.

import SwiftUI
import AVFoundation

struct LoopingClipPlayer: UIViewRepresentable {
    let url: URL

    func makeUIView(context: Context) -> PlayerLayerView {
        let view = PlayerLayerView()
        let player = AVQueuePlayer()
        player.isMuted = true
        context.coordinator.looper = AVPlayerLooper(player: player,
                                                    templateItem: AVPlayerItem(url: url))
        view.playerLayer.player = player
        view.playerLayer.videoGravity = .resizeAspectFill
        player.play()
        return view
    }

    func updateUIView(_ view: PlayerLayerView, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        var looper: AVPlayerLooper?
    }

    final class PlayerLayerView: UIView {
        override static var layerClass: AnyClass { AVPlayerLayer.self }
        var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }
    }
}

