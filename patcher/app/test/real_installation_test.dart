import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patcher/core.dart';
import 'package:patcher/main.dart';
import 'package:patcher/play_button.dart';

class FixedCore extends Core {
  @override
  void tick() {}
  @override
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]) => {'accepted': true};
}

void main() {
  testWidgets('a real installation snapshot draws the main screen with Play', (
    tester,
  ) async {
    final snapshot = jsonDecode(
      File('test/real_installation_snapshot.json').readAsStringSync(),
    ) as Map<String, dynamic>;
    tester.view.physicalSize = const Size(960, 780);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final core = FixedCore()..status.value = snapshot;
    await tester.pumpWidget(PatcherApp(core: core));
    await tester.pump(const Duration(milliseconds: 300));
    expect(tester.takeException(), isNull);
    expect(find.byType(PlayButton), findsOneWidget);
  });
}
