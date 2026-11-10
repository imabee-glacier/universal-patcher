import 'package:flutter/material.dart';

import 'tokens.dart';

/// What the Play button shows. The engine decides it; the button only draws it.
enum PlayState { available, starting, playing, unavailable }

PlayState playState(String? value) => switch (value) {
  'available' => PlayState.available,
  'starting' => PlayState.starting,
  'playing' => PlayState.playing,
  _ => PlayState.unavailable,
};

/// The one Play button: a large teal pill in the bottom slot of the main screen.
///
/// Available: teal, white label, hover darkens, press scales down slightly, Enter or Space
/// activates it when focused. Starting: a progress ring and "Starting...". Playing: a calm
/// mint pill with a live dot. Unavailable: muted, with the reason written underneath.
class PlayButton extends StatefulWidget {
  const PlayButton({
    super.key,
    required this.state,
    required this.onPressed,
    this.reason,
  });

  final PlayState state;
  final VoidCallback onPressed;

  /// Shown under the button: why it cannot be used, or what happened last time.
  final String? reason;

  @override
  State<PlayButton> createState() => _PlayButtonState();
}

class _PlayButtonState extends State<PlayButton>
    with SingleTickerProviderStateMixin {
  bool hovered = false;
  bool focused = false;
  bool pressed = false;
  late final AnimationController pulse = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1600),
  );

  bool get enabled => widget.state == PlayState.available;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    syncPulse();
  }

  @override
  void didUpdateWidget(PlayButton old) {
    super.didUpdateWidget(old);
    if (!enabled) pressed = false;
    syncPulse();
  }

  void syncPulse() {
    final still = MediaQuery.maybeDisableAnimationsOf(context) == true;
    if (widget.state == PlayState.playing && !still) {
      if (!pulse.isAnimating) pulse.repeat(reverse: true);
    } else {
      pulse
        ..stop()
        ..value = 1;
    }
  }

  @override
  void dispose() {
    pulse.dispose();
    super.dispose();
  }

  void press() {
    if (enabled) widget.onPressed();
  }

  @override
  Widget build(BuildContext context) {
    final length = motion(context, const Duration(milliseconds: 200));
    // Icons grow with the label at larger text sizes.
    final iconSize = MediaQuery.textScalerOf(context)
        .scale(24)
        .clamp(24, 48)
        .toDouble();
    final still = MediaQuery.maybeDisableAnimationsOf(context) == true;
    final (Color background, Color foreground) = switch (widget.state) {
      PlayState.available => (hovered || pressed ? tealHover : teal, onTeal),
      PlayState.starting => (teal, onTeal),
      PlayState.playing => (mint, ink),
      PlayState.unavailable => (line, muted),
    };
    final label = switch (widget.state) {
      PlayState.available => 'Play',
      PlayState.starting => 'Starting...',
      PlayState.playing => 'Playing',
      PlayState.unavailable => 'Play',
    };
    final Widget leading = switch (widget.state) {
      PlayState.starting when still => Icon(
        Icons.hourglass_empty_rounded,
        size: iconSize,
        color: foreground,
      ),
      PlayState.starting => SizedBox.square(
        dimension: 20,
        child: CircularProgressIndicator(
          key: const Key('play-progress'),
          strokeWidth: 2.4,
          color: foreground,
        ),
      ),
      PlayState.playing => FadeTransition(
        opacity: Tween(begin: .35, end: 1.0).animate(pulse),
        child: Container(
          key: const Key('play-live-dot'),
          width: 10,
          height: 10,
          margin: const EdgeInsets.all(7),
          decoration: const BoxDecoration(color: teal, shape: BoxShape.circle),
        ),
      ),
      _ => Icon(
        Icons.play_circle_outline_rounded,
        size: iconSize,
        color: foreground,
      ),
    };
    final content = Row(
      key: ValueKey(widget.state),
      mainAxisSize: MainAxisSize.min,
      children: [
        leading,
        const SizedBox(width: space8),
        Flexible(
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: foreground,
              fontSize: 18,
              fontWeight: FontWeight.w600,
              letterSpacing: .2,
            ),
          ),
        ),
      ],
    );
    final pill = AnimatedContainer(
      duration: length,
      curve: Curves.easeOut,
      constraints: const BoxConstraints(minWidth: 184, minHeight: 56),
      padding: const EdgeInsets.symmetric(
        vertical: space16,
        horizontal: space24,
      ),
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(radiusPill),
        boxShadow: enabled
            ? [
                BoxShadow(
                  color: ink.withAlpha(hovered ? 0x30 : 0x1f),
                  blurRadius: hovered ? 20 : 14,
                  spreadRadius: -4,
                  offset: Offset(0, hovered ? 8 : 6),
                ),
              ]
            : const [],
      ),
      child: Center(
        widthFactor: 1,
        child: AnimatedSwitcher(
          duration: length,
          switchInCurve: Curves.easeOut,
          switchOutCurve: Curves.easeIn,
          child: content,
        ),
      ),
    );
    // A visible focus ring, separated from the pill by a gap of the background colour.
    final ring = AnimatedContainer(
      duration: length,
      padding: const EdgeInsets.all(3),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(radiusPill),
        border: Border.all(color: focused ? ink : Colors.transparent, width: 2),
      ),
      child: pill,
    );
    final button = Semantics(
      button: true,
      enabled: enabled,
      label: switch (widget.state) {
        PlayState.available => 'Play The Sims 4',
        PlayState.starting => 'The Sims 4 is starting',
        PlayState.playing => 'The Sims 4 is running',
        PlayState.unavailable => 'Play, unavailable',
      },
      excludeSemantics: true,
      child: FocusableActionDetector(
        key: const Key('play-button'),
        enabled: enabled,
        mouseCursor: enabled
            ? SystemMouseCursors.click
            : SystemMouseCursors.basic,
        onShowFocusHighlight: (v) => setState(() => focused = v),
        onShowHoverHighlight: (v) => setState(() => hovered = v),
        actions: {
          ActivateIntent: CallbackAction<ActivateIntent>(
            onInvoke: (_) {
              press();
              return null;
            },
          ),
        },
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapDown: enabled ? (_) => setState(() => pressed = true) : null,
          onTapCancel: () => setState(() => pressed = false),
          onTapUp: enabled ? (_) => setState(() => pressed = false) : null,
          onTap: enabled ? press : null,
          child: AnimatedScale(
            scale: pressed ? .98 : 1,
            duration: motion(context, const Duration(milliseconds: 120)),
            child: ring,
          ),
        ),
      ),
    );
    final reason = widget.reason;
    final failed = widget.state == PlayState.available && reason != null;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Tooltip(
          message: switch (widget.state) {
            PlayState.available => 'Start The Sims 4',
            PlayState.starting => 'Waiting for the game to open',
            PlayState.playing => 'The game is running',
            PlayState.unavailable => reason ?? 'Play is unavailable',
          },
          child: button,
        ),
        const SizedBox(height: space8),
        // One line is always reserved, so the button does not move when a reason appears.
        if (reason == null)
          SizedBox(height: MediaQuery.textScalerOf(context).scale(13) * 1.4),
        if (reason != null) ...[
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Row(
              key: const Key('play-reason'),
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 1),
                  child: Icon(
                    failed
                        ? Icons.error_outline_rounded
                        : Icons.info_outline_rounded,
                    size: 16,
                    color: failed ? warning : muted,
                  ),
                ),
                const SizedBox(width: space8),
                Flexible(
                  child: Text(
                    reason,
                    style: TextStyle(
                      color: failed ? warning : muted,
                      fontSize: 13,
                      height: 1.4,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ],
    );
  }
}
