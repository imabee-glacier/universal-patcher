import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';

typedef _SnapshotNative = Pointer<Utf8> Function();
typedef _CommandNative = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _FreeNative = Void Function(Pointer<Utf8>);

abstract class Core {
  final status = ValueNotifier<Map<String, dynamic>>({});
  final progress = ValueNotifier<Map<String, dynamic>>({});
  void tick();
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]);
  void dispose() {
    status.dispose();
    progress.dispose();
  }
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
    _command = library
        .lookupFunction<_CommandNative, Pointer<Utf8> Function(Pointer<Utf8>)>(
          'patcher_command',
        );
    _free = library.lookupFunction<_FreeNative, void Function(Pointer<Utf8>)>(
      'patcher_free',
    );
  }
  late final Pointer<Utf8> Function() _snapshot;
  late final Pointer<Utf8> Function(Pointer<Utf8>) _command;
  late final void Function(Pointer<Utf8>) _free;
  String _lastStatus = '', _lastProgress = '';
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
    final data = _read(_snapshot());
    final numbers = <String, dynamic>{
      for (final key in ['bytes_done', 'bytes_total', 'bytes_per_second'])
        key: data.remove(key),
    };
    data.remove('revision');
    final signature = jsonEncode(data);
    final progressSignature = jsonEncode(numbers);
    if (signature != _lastStatus) {
      _lastStatus = signature;
      status.value = data;
    }
    if (progressSignature != _lastProgress) {
      _lastProgress = progressSignature;
      progress.value = numbers;
    }
  }

  @override
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]) {
    final input = jsonEncode({'id': 'ui-${++_id}', 'command': name, ...args})
        .toNativeUtf8();
    try {
      return _read(_command(input));
    } finally {
      malloc.free(input);
    }
  }
}
