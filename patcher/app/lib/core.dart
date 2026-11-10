import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/widgets.dart';

typedef _SnapshotNative = Pointer<Utf8> Function();
typedef _RevisionNative = Uint64 Function();
typedef _CommandNative = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _FreeNative = Void Function(Pointer<Utf8>);

/// The engine as the interface sees it: one snapshot notifier and a command call.
abstract class Core {
  /// The latest snapshot. It changes only when the engine's revision moved.
  final status = ValueNotifier<Map<String, dynamic>>({});

  /// Called every frame; cheap when nothing changed.
  void tick();
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]);
  void dispose() => status.dispose();
}

class NativeCore extends Core {
  NativeCore() {
    final override = Platform.environment['PATCHER_CORE_LIBRARY'];
    final executable = File(Platform.resolvedExecutable).parent.path;
    final library = DynamicLibrary.open(
      override ??
          (Platform.isWindows
              ? '$executable\\patcher_core.dll'
              : Platform.isMacOS
              ? '$executable/../Frameworks/libpatcher_core.dylib'
              : '$executable/lib/libpatcher_core.so'),
    );
    _snapshot = library
        .lookupFunction<_SnapshotNative, Pointer<Utf8> Function()>(
          'patcher_snapshot',
        );
    _revision = library.lookupFunction<_RevisionNative, int Function()>(
      'patcher_revision',
    );
    _command = library
        .lookupFunction<_CommandNative, Pointer<Utf8> Function(Pointer<Utf8>)>(
          'patcher_command',
        );
    _free = library.lookupFunction<_FreeNative, void Function(Pointer<Utf8>)>(
      'patcher_free',
    );
  }
  late final Pointer<Utf8> Function() _snapshot;
  late final int Function() _revision;
  late final Pointer<Utf8> Function(Pointer<Utf8>) _command;
  late final void Function(Pointer<Utf8>) _free;
  int? _seen;
  int _id = 0;
  Map<String, dynamic> _read(Pointer<Utf8> pointer) {
    try {
      return jsonDecode(pointer.toDartString()) as Map<String, dynamic>;
    } finally {
      _free(pointer);
    }
  }

  @override
  void tick() {
    if (_revision() == _seen) return;
    final data = _read(_snapshot());
    _seen = (data['revision'] as num?)?.toInt();
    status.value = data;
  }

  @override
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]) {
    final input = jsonEncode({'id': 'ui-${++_id}', 'command': name, ...args})
        .toNativeUtf8();
    try {
      final result = _read(_command(input));
      tick();
      return result;
    } finally {
      malloc.free(input);
    }
  }
}

/// Rebuilds [builder] only when the part of the snapshot it reads changes.
class Select<T> extends StatefulWidget {
  const Select({
    super.key,
    required this.core,
    required this.selector,
    required this.builder,
  });
  final Core core;
  final T Function(Map<String, dynamic> snapshot) selector;
  final Widget Function(BuildContext context, T value) builder;
  @override
  State<Select<T>> createState() => _SelectState<T>();
}

class _SelectState<T> extends State<Select<T>> {
  late T value;
  late String signature;

  @override
  void initState() {
    super.initState();
    value = widget.selector(widget.core.status.value);
    signature = jsonEncode(value);
    widget.core.status.addListener(changed);
  }

  @override
  void didUpdateWidget(Select<T> old) {
    super.didUpdateWidget(old);
    if (old.core != widget.core) {
      old.core.status.removeListener(changed);
      widget.core.status.addListener(changed);
    }
    value = widget.selector(widget.core.status.value);
    signature = jsonEncode(value);
  }

  void changed() {
    final next = widget.selector(widget.core.status.value);
    final nextSignature = jsonEncode(next);
    if (nextSignature != signature) {
      setState(() {
        value = next;
        signature = nextSignature;
      });
    }
  }

  @override
  void dispose() {
    widget.core.status.removeListener(changed);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.builder(context, value);
}
