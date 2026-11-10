import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patcher/main.dart';
import 'package:patcher/core.dart';
import 'package:patcher/collection.dart';

class FakeCore extends Core {
  final commands = <String>[];
  final arguments = <Map<String, dynamic>>[];
  @override
  void tick() {}
  @override
  Map<String, dynamic> command(
    String name, [
    Map<String, dynamic> args = const {},
  ]) {
    commands.add(name);
    arguments.add(args);
    return {'accepted': true};
  }
}

Map<String, dynamic> readyWithPacks() => {
  'state': 'ready',
  'ready': true,
  'title': 'Ready',
  'detail': 'Verified',
  'actions': ['Verify', 'SetPolicy'],
  'policy_revision': 7,
  'preferences': {
    'future_packs': false,
    'background': false,
    'metered': false,
    'bandwidth_bytes_per_second': 0,
  },
  'packs': [
    {
      'id': 'EP01',
      'name': 'Expansion',
      'selected': true,
      'answered': true,
      'deferred': false,
      'bytes': 1000,
    },
    {
      'id': 'SP02',
      'name': 'New stuff',
      'selected': false,
      'answered': false,
      'deferred': false,
      'bytes': 2000,
    },
  ],
  'next_action': '',
  'online': 'Unavailable',
};

void main() {
  testWidgets('controls submit commands; unavailable launch stays disabled', (
    tester,
  ) async {
    final core = FakeCore();
    core.status.value = {
      'state': 'ready',
      'ready': true,
      'title': 'Everything is in place',
      'detail': 'Verified fixture',
      'fixture': true,
      'actions': ['Verify', 'SetPolicy'],
      'packs': [],
      'next_action': 'Fixture mode',
      'online': 'Unavailable',
    };
    await tester.pumpWidget(PatcherApp(core: core));
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('primary-action')))
          .onPressed,
      isNull,
    );
    await tester.tap(find.byKey(const Key('verify-action')));
    expect(core.commands, ['Verify']);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('saving packs never answers an untouched new pack', (
    tester,
  ) async {
    final core = FakeCore()..status.value = readyWithPacks();
    await tester.pumpWidget(PatcherApp(core: core));
    expect(find.textContaining('1 new pack to review'), findsOneWidget);
    await tester.tap(find.text('Your collection'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    await tester.tap(find.byKey(const Key('save-packs')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(core.commands.last, 'SetPolicy');
    expect(core.arguments.last['revision'], 7);
    expect(core.arguments.last['dlc'], {'EP01': true});
    expect(core.arguments.last['deferred'], isEmpty);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('decide later defers only untouched new packs', (tester) async {
    final core = FakeCore()..status.value = readyWithPacks();
    await tester.pumpWidget(PatcherApp(core: core));
    await tester.tap(find.text('Your collection'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    await tester.tap(find.byKey(const Key('defer-packs')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(core.arguments.last['dlc'], {'EP01': true});
    expect(core.arguments.last['deferred'], ['SP02']);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('settings send every preference with the revision', (
    tester,
  ) async {
    final core = FakeCore()..status.value = readyWithPacks();
    await tester.pumpWidget(PatcherApp(core: core));
    await tester.tap(find.byTooltip('Settings'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    await tester.tap(find.byKey(const Key('pref-future')));
    await tester.tap(find.text('5 MB/s'));
    await tester.pump();
    await tester.ensureVisible(find.byKey(const Key('save-preferences')));
    await tester.tap(find.byKey(const Key('save-preferences')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(core.commands.last, 'SetPolicy');
    expect(core.arguments.last, {
      'revision': 7,
      'preferences': {
        'future_packs': true,
        'background': false,
        'metered': false,
        'bandwidth_bytes_per_second': 5242880,
      },
    });
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('small window and long content remain usable', (tester) async {
    tester.view.physicalSize = const Size(720, 560);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final core = FakeCore();
    for (final state in [
      'setup',
      'downloading',
      'paused',
      'needs_attention',
      'waiting',
      'ready',
    ]) {
      core.status.value = {
        'state': state,
        'title':
            'A long translated maintenance status that must remain readable',
        'detail': 'A long pack name and a detailed recovery explanation. ' * 5,
        'actions': ['Resume'],
        'packs': [],
        'online': 'Unavailable',
        'next_action': 'Resume to finish verification.',
      };
      for (final scale in [1.0, 2.0]) {
        await tester.pumpWidget(
          MediaQuery(
            data: MediaQueryData(
              size: const Size(720, 560),
              textScaler: TextScaler.linear(scale),
            ),
            child: PatcherApp(core: core),
          ),
        );
        await tester.pump();
        expect(tester.takeException(), isNull, reason: '$state at $scale');
        await tester.ensureVisible(find.byKey(const Key('primary-action')));
        await tester.pump();
        expect(
          find.byKey(const Key('primary-action')).hitTestable(),
          findsOneWidget,
          reason: 'primary action reachable in $state at $scale',
        );
      }
    }
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets(
    'a found game is used; the folder picker leads only when none is found',
    (tester) async {
      final core = FakeCore();
      Map<String, dynamic> setup(Map<String, dynamic> extra) => {
        'state': 'setup',
        'title': 'Game found',
        'detail': '',
        'actions': ['Discover', 'ConfirmInstall'],
        'packs': [],
        'online': 'Unavailable',
        'next_action': '',
        ...extra,
      };
      final pickerLabel = find.text('Choose game folder');

      // One installation found and saved: no picker as the main action.
      core.status.value = setup({
        'path': r'C:\Program Files\EA Games\The Sims 4',
      });
      await tester.pumpWidget(PatcherApp(core: core));
      await tester.pump();
      expect(
        find.text(r'C:\Program Files\EA Games\The Sims 4'),
        findsOneWidget,
      );
      expect(pickerLabel, findsNothing);
      expect(find.byKey(const Key('other-folder')), findsOneWidget);

      // Several found: choose from the list; the picker is secondary.
      core.status.value = setup({
        'title': 'Choose your installation',
        'candidates': [
          {'path': r'C:\A', 'source': 'EA app'},
          {'path': r'D:\B', 'source': 'Steam library'},
        ],
      });
      await tester.pump();
      expect(pickerLabel, findsNothing);
      await tester.ensureVisible(find.text(r'D:\B'));
      await tester.tap(find.text(r'D:\B'));
      expect(core.commands.last, 'ConfirmInstall');
      expect(core.arguments.last, {'path': r'D:\B'});

      // Nothing found: the picker is the main action.
      core.status.value = setup({'title': 'Choose your game folder'});
      await tester.pump();
      expect(pickerLabel, findsOneWidget);
      expect(find.byKey(const Key('other-folder')), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
  Map<String, dynamic> installedGame({
    List<Map<String, dynamic>>? packs,
    List<String>? actions,
  }) => {
    'state': 'installed',
    'title': 'Your game',
    'detail': '2 of 4 packs installed',
    'path': r'C:\Program Files\EA Games\The Sims 4',
    'actions':
        actions ??
        ['Verify', 'Launch', 'SetPolicy', 'Discover', 'ConfirmInstall'],
    'policy_revision': 3,
    'languages': ['ENG_US', 'GER_DE'],
    'language': 'ENG_US',
    'background_supported': false,
    'preferences': {
      'future_packs': false,
      'background': false,
      'metered': false,
      'bandwidth_bytes_per_second': 0,
    },
    'online': 'Unavailable',
    'next_action': '',
    'packs':
        packs ??
        [
          {
            'id': 'EP01',
            'name': 'Get to Work',
            'kind': 'expansion',
            'status': 'installed',
            'selected': true,
            'answered': true,
            'deferred': false,
            'bytes': 0,
            'installed_bytes': 4000000000,
            'issues': [],
          },
          {
            'id': 'GP05',
            'name': 'Parenthood',
            'kind': 'game',
            'status': 'incomplete',
            'selected': true,
            'answered': true,
            'deferred': false,
            'bytes': 0,
            'installed_bytes': 900000000,
            'issues': ['Missing: GP05/Version.ini'],
          },
          {
            'id': 'SP20',
            'name': 'Throwback Fit',
            'kind': 'kit',
            'status': 'not_installed',
            'selected': false,
            'answered': true,
            'deferred': false,
            'bytes': 0,
            'installed_bytes': 0,
            'issues': [],
          },
          {
            'id': 'FP01',
            'name': 'Holiday Celebration',
            'kind': 'free',
            'status': 'not_installed',
            'selected': false,
            'answered': true,
            'deferred': false,
            'bytes': 0,
            'installed_bytes': 0,
            'issues': [],
          },
        ],
  };

  testWidgets('an installed game offers Play, Check files and its collection', (
    tester,
  ) async {
    final core = FakeCore();
    core.status.value = installedGame();
    await tester.pumpWidget(PatcherApp(core: core));
    await tester.pump();
    expect(find.text('Installed'), findsOneWidget, reason: 'state label');
    expect(find.text('2 of 4 installed · 1 needs attention'), findsOneWidget);
    await tester.tap(find.byKey(const Key('primary-action')));
    expect(core.commands.last, 'Launch');
    await tester.tap(find.byKey(const Key('verify-action')));
    expect(core.commands.last, 'Verify');
    expect(find.text('Check files'), findsOneWidget);
    expect(find.byKey(const Key('accept-check')), findsNothing);
    core.status.value = installedGame(
      actions: ['Verify', 'Launch', 'SetPolicy', 'AcceptCheck'],
    );
    await tester.pump();
    await tester.tap(find.byKey(const Key('accept-check')));
    expect(core.commands.last, 'AcceptCheck');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('unanswered installed packs lead with a review', (tester) async {
    final core = FakeCore();
    final packs = List<Map<String, dynamic>>.from(installedGame()['packs'])
      ..[0] = {...installedGame()['packs'][0], 'answered': false};
    core.status.value = installedGame(packs: packs);
    await tester.pumpWidget(PatcherApp(core: core));
    await tester.pump();
    expect(find.text('Review packs'), findsOneWidget);
    expect(
      find.byKey(const Key('play-action')),
      findsOneWidget,
      reason: 'Play stays available',
    );
    await tester.tap(find.byKey(const Key('primary-action')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(find.byKey(const Key('review-banner')), findsOneWidget);
    expect(find.text('Confirm my packs'), findsOneWidget);
    await tester.tap(find.byKey(const Key('save-packs')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(core.commands.last, 'SetPolicy');
    expect(core.arguments.last['dlc'], {
      'EP01': true,
      'GP05': true,
      'SP20': false,
      'FP01': false,
    });
    expect(core.arguments.last['revision'], 3);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('the collection filters, explains problems and saves choices', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    Map<String, bool>? saved;
    await tester.pumpWidget(
      MaterialApp(
        home: CollectionPage(
          status: installedGame(),
          save: (dlc) {
            saved = dlc;
            return null;
          },
        ),
      ),
    );
    await tester.pump();
    expect(find.byKey(const Key('stat-installed')), findsOneWidget);
    expect(find.text('1 need attention'), findsOneWidget);
    for (final id in ['EP01', 'GP05', 'SP20', 'FP01']) {
      expect(find.byKey(Key('pack-$id')), findsOneWidget, reason: id);
    }
    // Save stays off until something changes.
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('save-packs')))
          .onPressed,
      isNull,
    );

    await tester.tap(find.byKey(const Key('state-attention')));
    await tester.pump();
    expect(find.byKey(const Key('pack-GP05')), findsOneWidget);
    expect(find.byKey(const Key('pack-EP01')), findsNothing);
    await tester.tap(find.byKey(const Key('issues-GP05')));
    await tester.pumpAndSettle();
    expect(find.text('Missing: GP05/Version.ini'), findsOneWidget);
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('state-all')));
    await tester.tap(find.byKey(const Key('kind-kit')));
    await tester.pump();
    expect(find.byKey(const Key('pack-SP20')), findsOneWidget);
    expect(find.byKey(const Key('pack-GP05')), findsNothing);
    await tester.tap(find.byKey(const Key('pack-SP20')));
    await tester.pump();

    await tester.tap(find.byKey(const Key('kind-all')));
    await tester.enterText(find.byKey(const Key('pack-search')), 'holiday');
    await tester.pump();
    expect(find.byKey(const Key('pack-FP01')), findsOneWidget);
    expect(find.byKey(const Key('pack-EP01')), findsNothing);

    await tester.tap(find.byKey(const Key('save-packs')));
    await tester.pumpAndSettle();
    expect(saved, {'EP01': true, 'GP05': true, 'SP20': true, 'FP01': false});
    expect(tester.takeException(), isNull);
  });

  testWidgets('settings offer installed languages and hide what cannot work', (
    tester,
  ) async {
    final core = FakeCore();
    core.status.value = installedGame();
    await tester.pumpWidget(PatcherApp(core: core));
    await tester.pump();
    await tester.tap(find.byTooltip('Settings'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(find.byKey(const Key('lang-GER_DE')), findsOneWidget);
    expect(find.text('Deutsch'), findsOneWidget);
    expect(find.byKey(const Key('pref-background')), findsNothing);
    expect(find.byKey(const Key('downloads-unavailable')), findsOneWidget);
    await tester.tap(find.byKey(const Key('lang-GER_DE')));
    await tester.pump();
    await tester.ensureVisible(find.byKey(const Key('save-preferences')));
    await tester.tap(find.byKey(const Key('save-preferences')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(core.commands.last, 'SetPolicy');
    expect(core.arguments.last, {'revision': 3, 'language': 'GER_DE'});
    await tester.pumpWidget(const SizedBox());
  });
}
