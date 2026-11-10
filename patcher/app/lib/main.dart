import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:file_selector/file_selector.dart';

import 'collection.dart';
import 'core.dart';
import 'play_button.dart';
import 'tokens.dart';

export 'tokens.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    runApp(PatcherApp(core: NativeCore()));
  } catch (error) {
    runApp(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: Padding(
              padding: const EdgeInsets.all(space32),
              child: Text(
                'The maintenance engine could not start.\nReinstall the patcher to restore its native library.\n\n$error',
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class PatcherApp extends StatelessWidget {
  const PatcherApp({super.key, required this.core, this.fontFamily});
  final Core core;

  /// The system sans-serif unless a test names a font.
  final String? fontFamily;

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Universal Patcher',
    debugShowCheckedModeBanner: false,
    theme: ThemeData(
      useMaterial3: true,
      fontFamily: fontFamily,
      scaffoldBackgroundColor: paper,
      colorScheme: ColorScheme.fromSeed(
        seedColor: teal,
        primary: teal,
        onPrimary: onTeal,
        surface: paper,
        onSurface: ink,
        error: error,
      ),
      textTheme: const TextTheme(
        bodyMedium: TextStyle(color: ink, fontSize: 15, height: 1.5),
        bodySmall: TextStyle(color: muted, fontSize: 12, height: 1.5),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: teal,
          foregroundColor: onTeal,
          minimumSize: const Size(120, 48),
          padding: const EdgeInsets.symmetric(horizontal: space24),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(radiusPill),
          ),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          foregroundColor: ink,
          minimumSize: const Size(100, 44),
          padding: const EdgeInsets.symmetric(horizontal: space16),
          side: const BorderSide(color: line),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(radiusPill),
          ),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(foregroundColor: teal),
      ),
      dividerColor: line,
      snackBarTheme: const SnackBarThemeData(backgroundColor: ink),
    ),
    home: Home(core: core),
  );
}

/// Snapshot fields that change while work progresses; the main screen does not rebuild for them.
const progressKeys = {
  'revision',
  'bytes_done',
  'bytes_total',
  'bytes_per_second',
  'files_done',
  'files_total',
  'detail',
};

/// How often the main screen was built; tests check that progress alone does not rebuild it.
@visibleForTesting
int homeBuilds = 0;

List<String> actionsOf(Map<String, dynamic> s) =>
    List<String>.from(s['actions'] ?? const []);

String labelOf(Map<String, dynamic> s, String action) =>
    (s['labels'] as Map?)?[action]?.toString() ?? action;

String? unavailableOf(Map<String, dynamic> s, String action) =>
    (s['unavailable'] as Map?)?[action]?.toString();

class Home extends StatefulWidget {
  const Home({super.key, required this.core});
  final Core core;
  @override
  State<Home> createState() => _HomeState();
}

class _HomeState extends State<Home> with SingleTickerProviderStateMixin {
  late final Ticker _ticker;
  bool details = false;
  @override
  void initState() {
    super.initState();
    widget.core.tick();
    _ticker = createTicker((_) => widget.core.tick())..start();
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  void send(String command, [Map<String, dynamic> args = const {}]) {
    final result = widget.core.command(command, args);
    if (result['accepted'] != true && mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            result['error']?.toString() ?? 'That action is unavailable.',
          ),
        ),
      );
    }
  }

  Future<void> chooseFolder() async {
    final path = await getDirectoryPath(
      confirmButtonText: 'Use this installation',
    );
    if (path != null && mounted) {
      send('ConfirmInstall', {'path': path});
    }
  }

  /// Runs an action the engine offered. Choosing a folder opens the picker first.
  void run(String action) {
    if (action == 'ConfirmInstall') {
      chooseFolder();
    } else {
      send(action);
    }
  }

  void openCollection(Map<String, dynamic> s) {
    final actions = actionsOf(s);
    if (actions.contains('ViewPacks')) {
      Navigator.of(context).push(
        MaterialPageRoute<void>(builder: (_) => CollectionPage(status: s)),
      );
    } else if (actions.contains('SetPolicy')) {
      packSheet(s);
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: Select<Map<String, dynamic>>(
        core: widget.core,
        selector: (s) => {
          for (final e in s.entries)
            if (!progressKeys.contains(e.key)) e.key: e.value,
        },
        builder: (context, s) => LayoutBuilder(
          builder: (context, box) {
            homeBuilds++;
            final wide = box.maxWidth > 900;
            final side = wide ? space48 : space32;
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: EdgeInsets.fromLTRB(side, space16, side - 8, 0),
                  child: header(s),
                ),
                Expanded(
                  child: SingleChildScrollView(
                    padding: EdgeInsets.fromLTRB(side, space16, side, space24),
                    child: centre(s),
                  ),
                ),
                ConstrainedBox(
                  constraints: BoxConstraints(maxHeight: box.maxHeight * .5),
                  child: Container(
                    decoration: const BoxDecoration(
                      color: surface,
                      border: Border(top: BorderSide(color: line)),
                    ),
                    child: SingleChildScrollView(
                      padding: EdgeInsets.fromLTRB(
                        side,
                        space16,
                        side,
                        space16,
                      ),
                      child: bottom(s),
                    ),
                  ),
                ),
              ],
            );
          },
        ),
      ),
    ),
  );

  Widget header(Map<String, dynamic> s) {
    final update = s['app_update'];
    return Row(
      children: [
        Container(
          width: 40,
          height: 40,
          decoration: BoxDecoration(
            color: teal,
            borderRadius: BorderRadius.circular(12),
          ),
          child: const Icon(Icons.spa_outlined, color: onTeal, size: 22),
        ),
        const SizedBox(width: space16),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'The Sims 4',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: ink,
                  fontSize: 16,
                  fontWeight: FontWeight.w600,
                ),
              ),
              StatusLabel(status: s),
            ],
          ),
        ),
        if (update != null)
          Padding(
            padding: const EdgeInsets.only(left: space8),
            child: Tooltip(
              message: 'Universal Patcher $update starts next time',
              child: const Chip(
                key: Key('update-ready'),
                avatar: Icon(Icons.system_update_alt_rounded, size: 16),
                label: Text('Update ready - restart to apply'),
                backgroundColor: mint,
                side: BorderSide.none,
                labelStyle: TextStyle(color: ink, fontSize: 12),
              ),
            ),
          ),
        if (s['fixture'] == true)
          const Padding(
            padding: EdgeInsets.only(left: space8),
            child: BadgeLabel(text: 'TEST INSTALLATION'),
          ),
        const SizedBox(width: space8),
        IconButton(
          tooltip: 'Settings',
          onPressed: () => settings(s),
          icon: const Icon(Icons.tune_rounded, color: ink),
        ),
      ],
    );
  }

  Widget centre(Map<String, dynamic> s) {
    final actions = actionsOf(s);
    final primary = s['primary']?.toString();
    final play = s['play']?.toString() ?? 'hidden';
    final playVisible = play != 'hidden';
    // Settings holds folder and search actions while a game is shown.
    final inSettings = {
      'Launch',
      'SetPolicy',
      'ViewPacks',
      'CheckAppUpdate',
      if (playVisible && s['state'] != 'confirm_location') ...[
        'Discover',
        'ConfirmInstall',
      ],
      // The bottom slot shows the leading action when there is no Play button.
      if (!playVisible && primary != null) primary,
    };
    final beside = {'Pause', 'Resume', 'Cancel'};
    final showProgress = s['show_progress'] == true;
    final cardActions = actions
        .where((a) => !inSettings.contains(a))
        .where((a) => !(showProgress && beside.contains(a)))
        .toList();
    final candidates = List<Map<String, dynamic>>.from(
      s['candidates'] ?? const [],
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          padding: const EdgeInsets.all(space24),
          decoration: BoxDecoration(
            color: surface,
            borderRadius: BorderRadius.circular(radiusMedium + 4),
            border: Border.all(color: line),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              AnimatedSwitcher(
                duration: motion(context),
                child: Text(
                  s['title']?.toString() ?? 'Finding your game',
                  key: ValueKey(s['title']),
                  style: const TextStyle(
                    color: ink,
                    fontSize: 30,
                    fontWeight: FontWeight.w500,
                    letterSpacing: -.8,
                    height: 1.15,
                  ),
                ),
              ),
              const SizedBox(height: space8),
              Select<String>(
                core: widget.core,
                selector: (s) => s['detail']?.toString() ?? '',
                builder: (context, detail) => Text(
                  detail,
                  key: const Key('detail'),
                  maxLines: showProgress ? 2 : null,
                  overflow: showProgress ? TextOverflow.ellipsis : null,
                  style: const TextStyle(
                    color: muted,
                    fontSize: 15,
                    height: 1.5,
                  ),
                ),
              ),
              if (showProgress) ...[
                const SizedBox(height: space16),
                ProgressView(
                  core: widget.core,
                  controls: [
                    for (final a in actions.where(beside.contains))
                      a == 'Cancel'
                          ? TextButton(
                              key: Key('action-$a'),
                              onPressed: () => run(a),
                              child: Text(labelOf(s, a)),
                            )
                          : OutlinedButton.icon(
                              key: Key('action-$a'),
                              onPressed: () => run(a),
                              icon: Icon(
                                a == 'Pause'
                                    ? Icons.pause_rounded
                                    : Icons.play_arrow_rounded,
                                size: 18,
                              ),
                              label: Text(labelOf(s, a)),
                            ),
                  ],
                ),
              ],
              if (cardActions.isNotEmpty) ...[
                const SizedBox(height: space16),
                Wrap(
                  spacing: space8,
                  runSpacing: space8,
                  children: [
                    for (final a in cardActions)
                      a == primary
                          ? FilledButton(
                              key: Key('action-$a'),
                              onPressed: () => run(a),
                              child: Text(labelOf(s, a)),
                            )
                          : OutlinedButton(
                              key: Key('action-$a'),
                              onPressed: () => run(a),
                              child: Text(labelOf(s, a)),
                            ),
                  ],
                ),
              ],
              if ((s['next_action']?.toString() ?? '').isNotEmpty) ...[
                const SizedBox(height: space16),
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Padding(
                      padding: EdgeInsets.only(top: 2),
                      child: Icon(
                        Icons.arrow_forward_rounded,
                        size: 16,
                        color: muted,
                      ),
                    ),
                    const SizedBox(width: space8),
                    Expanded(
                      child: Text(
                        s['next_action'].toString(),
                        key: const Key('next-action'),
                        style: const TextStyle(color: muted, fontSize: 13),
                      ),
                    ),
                  ],
                ),
              ],
            ],
          ),
        ),
        if (s['path'] != null) ...[
          const SizedBox(height: space16),
          Row(
            children: [
              const Icon(Icons.folder_outlined, size: 16, color: muted),
              const SizedBox(width: space8),
              Expanded(
                child: Text(
                  s['path'].toString(),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: muted, fontSize: 12),
                ),
              ),
            ],
          ),
        ],
        if (candidates.isNotEmpty && s['path'] == null) ...[
          const SizedBox(height: space8),
          for (final candidate in candidates)
            ListTile(
              leading: const Icon(Icons.folder_outlined),
              title: Text(candidate['path'].toString()),
              subtitle: Text(candidate['source'].toString()),
              trailing: const Icon(Icons.arrow_forward_rounded),
              onTap: () => send('ConfirmInstall', {'path': candidate['path']}),
            ),
        ],
        const SizedBox(height: space16),
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            key: const Key('details-toggle'),
            onPressed: () => setState(() => details = !details),
            icon: Icon(
              details ? Icons.expand_less_rounded : Icons.expand_more_rounded,
              size: 18,
            ),
            label: const Text('Details'),
          ),
        ),
        if (details)
          SelectionArea(
            child: Padding(
              padding: const EdgeInsets.all(space8),
              child: Text(
                [
                  if (s['error'] != null) s['error'].toString(),
                  ...List<dynamic>.from(s['details'] ?? const [])
                      .map((line) => line.toString()),
                  if ((s['checked'] as List?)?.isNotEmpty ?? false) ...[
                    'Locations checked:',
                    ...List<dynamic>.from(s['checked'])
                        .map((p) => p.toString()),
                  ],
                ].join('\n').ifEmpty('Nothing to report yet.'),
                style: const TextStyle(color: muted, fontSize: 12),
              ),
            ),
          ),
      ],
    );
  }

  Widget bottom(Map<String, dynamic> s) {
    final play = s['play']?.toString() ?? 'hidden';
    final primary = s['primary']?.toString();
    final actions = actionsOf(s);
    final Widget lead;
    if (play != 'hidden') {
      lead = PlayButton(
        state: playState(play),
        reason: s['play_reason']?.toString(),
        onPressed: () => send('Launch'),
      );
    } else if (primary != null && actions.contains(primary)) {
      lead = FilledButton.icon(
        key: const Key('primary-action'),
        style: FilledButton.styleFrom(
          minimumSize: const Size(184, 56),
          textStyle: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
        ),
        onPressed: () => run(primary),
        icon: Icon(switch (primary) {
          'ConfirmInstall' => Icons.folder_open_outlined,
          'Discover' => Icons.search_rounded,
          'Resume' => Icons.play_arrow_rounded,
          _ => Icons.arrow_forward_rounded,
        }),
        label: Text(labelOf(s, primary)),
      );
    } else {
      lead = const SizedBox.shrink();
    }
    final collection = CollectionTile(
      summary: s['collection']?.toString() ?? '',
      onTap:
          actions.contains('ViewPacks') ||
              (actions.contains('SetPolicy') &&
                  (s['packs'] as List? ?? const []).isNotEmpty)
          ? () => openCollection(s)
          : null,
    );
    final stacked = MediaQuery.textScalerOf(context).scale(16) > 22;
    if (stacked) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          lead,
          const SizedBox(height: space16),
          collection,
        ],
      );
    }
    // The lead keeps one width in every state, so the pack summary never moves.
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 360,
          child: Align(alignment: Alignment.topLeft, child: lead),
        ),
        const SizedBox(width: space24),
        Expanded(child: collection),
      ],
    );
  }

  void settings(Map<String, dynamic> s) {
    final actions = actionsOf(s);
    final saved = Map<String, dynamic>.from(s['preferences'] ?? const {});
    final languages = List<String>.from(s['languages'] ?? const []);
    String? language = s['language'] as String?;
    final downloads = s['downloads_supported'] == true;
    final background = s['background_supported'] == true;
    final canSave = actions.contains('SetPolicy');
    final prefs = <String, dynamic>{
      'future_packs': saved['future_packs'] == true,
      'background': saved['background'] == true,
      'metered': saved['metered'] == true,
      'bandwidth_bytes_per_second': saved['bandwidth_bytes_per_second'] ?? 0,
    };
    final updateReason = unavailableOf(s, 'CheckAppUpdate');
    final updateStatus = s['app_update_status']?.toString();
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      isScrollControlled: true,
      backgroundColor: surface,
      builder: (sheetContext) => StatefulBuilder(
        builder: (sheetContext, update) => SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(space32, space8, space32, space32),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'Settings',
                style: TextStyle(fontSize: 26, color: ink),
              ),
              const SizedBox(height: space16),
              if (languages.isNotEmpty) ...[
                const Text('Game language', style: TextStyle(color: ink)),
                const SizedBox(height: space8),
                Wrap(
                  spacing: space8,
                  runSpacing: space8,
                  children: [
                    for (final code in languages)
                      ChoiceChip(
                        key: Key('lang-$code'),
                        label: Text(languageName(code)),
                        selected: language == code,
                        onSelected: canSave
                            ? (_) => update(() => language = code)
                            : null,
                      ),
                  ],
                ),
                const SizedBox(height: space8),
                const Text(
                  'Languages installed with your game. The choice is saved for when language switching is validated; your game is not changed yet.',
                  style: TextStyle(color: muted, fontSize: 12),
                ),
                const SizedBox(height: space16),
              ],
              if (!downloads)
                const ListTile(
                  key: Key('downloads-unavailable'),
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(Icons.cloud_off_outlined),
                  title: Text('Downloads and background maintenance'),
                  subtitle: Text(
                    'Available once game releases are published. Until then the patcher checks your own files only.',
                  ),
                ),
              if (downloads && background)
                SwitchListTile(
                  key: const Key('pref-background'),
                  contentPadding: EdgeInsets.zero,
                  secondary: const Icon(Icons.schedule_outlined),
                  title: const Text('Maintain in the background'),
                  subtitle: s['background_note'] == null
                      ? null
                      : Text(s['background_note'].toString()),
                  value: prefs['background'] as bool,
                  onChanged: canSave
                      ? (v) => update(() => prefs['background'] = v)
                      : null,
                ),
              if (downloads) ...[
                SwitchListTile(
                  key: const Key('pref-future'),
                  contentPadding: EdgeInsets.zero,
                  secondary: const Icon(Icons.new_releases_outlined),
                  title: const Text('Include future packs automatically'),
                  subtitle: const Text(
                    'New packs download without asking. This uses more disk space and bandwidth.',
                  ),
                  value: prefs['future_packs'] as bool,
                  onChanged: canSave
                      ? (v) => update(() => prefs['future_packs'] = v)
                      : null,
                ),
                SwitchListTile(
                  key: const Key('pref-metered'),
                  contentPadding: EdgeInsets.zero,
                  secondary: const Icon(Icons.signal_cellular_alt_outlined),
                  title: const Text('Allow metered connections'),
                  subtitle: const Text(
                    'Background downloads otherwise wait for an unmetered network.',
                  ),
                  value: prefs['metered'] as bool,
                  onChanged: canSave
                      ? (v) => update(() => prefs['metered'] = v)
                      : null,
                ),
                const SizedBox(height: space8),
                const Text(
                  'Download speed limit',
                  style: TextStyle(color: ink),
                ),
                const SizedBox(height: space8),
                Wrap(
                  spacing: space8,
                  runSpacing: space8,
                  children: [
                    for (final (label, value) in bandwidthChoices)
                      ChoiceChip(
                        label: Text(label),
                        selected: prefs['bandwidth_bytes_per_second'] == value,
                        onSelected: canSave
                            ? (_) => update(
                                () =>
                                    prefs['bandwidth_bytes_per_second'] = value,
                              )
                            : null,
                      ),
                  ],
                ),
              ],
              const SizedBox(height: space16),
              const ListTile(
                contentPadding: EdgeInsets.zero,
                leading: Icon(Icons.person_outline),
                title: Text('Online sign-in'),
                subtitle: Text(
                  'Unavailable until the sign-in handoff is verified.',
                ),
              ),
              ListTile(
                key: const Key('patcher-update'),
                contentPadding: EdgeInsets.zero,
                leading: const Icon(Icons.system_update_alt_rounded),
                title: Text(
                  s['app_update'] != null
                      ? 'Update ready - restart to apply'
                      : 'Patcher updates',
                ),
                subtitle: Text(
                  updateStatus ?? updateReason ?? 'Checks for a newer patcher.',
                ),
                trailing: actions.contains('CheckAppUpdate')
                    ? TextButton(
                        key: const Key('action-CheckAppUpdate'),
                        onPressed: () {
                          Navigator.pop(sheetContext);
                          send('CheckAppUpdate');
                        },
                        child: Text(labelOf(s, 'CheckAppUpdate')),
                      )
                    : null,
              ),
              const SizedBox(height: space16),
              Wrap(
                spacing: space16,
                runSpacing: space16,
                children: [
                  if (canSave)
                    FilledButton(
                      key: const Key('save-preferences'),
                      onPressed: () {
                        Navigator.pop(sheetContext);
                        send('SetPolicy', {
                          'revision': s['policy_revision'],
                          if (downloads) 'preferences': prefs,
                          if (languages.isNotEmpty) 'language': language,
                        });
                      },
                      child: Text(labelOf(s, 'SetPolicy')),
                    ),
                  if (actions.contains('ConfirmInstall'))
                    OutlinedButton.icon(
                      key: const Key('settings-ConfirmInstall'),
                      onPressed: () {
                        Navigator.pop(sheetContext);
                        chooseFolder();
                      },
                      icon: const Icon(Icons.folder_open_outlined),
                      label: Text(labelOf(s, 'ConfirmInstall')),
                    ),
                  if (actions.contains('Discover'))
                    OutlinedButton.icon(
                      key: const Key('settings-Discover'),
                      onPressed: () {
                        Navigator.pop(sheetContext);
                        send('Discover');
                      },
                      icon: const Icon(Icons.search_rounded),
                      label: Text(labelOf(s, 'Discover')),
                    ),
                ],
              ),
              if ((s['mode_note']?.toString() ?? '').isNotEmpty) ...[
                const SizedBox(height: space16),
                Text(
                  s['mode_note'].toString(),
                  style: const TextStyle(color: muted, fontSize: 12),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }

  /// Pack choices for a release: only offered when the engine accepts them (SetPolicy).
  void packSheet(Map<String, dynamic> s) {
    final packs = List<Map<String, dynamic>>.from(s['packs']);
    // Only answered packs start with a decision; new packs stay undecided until touched.
    final selections = <String, bool>{
      for (final p in packs)
        if (p['answered'] == true) p['id']: p['selected'] == true,
    };
    final deferred = <String>{
      for (final p in packs)
        if (p['deferred'] == true) p['id'] as String,
    };
    bool undecided(Map<String, dynamic> p) => p['undecided'] == true;
    packs.sort(
      (a, b) => (undecided(a) ? 0 : 1).compareTo(undecided(b) ? 0 : 1),
    );
    String query = '';
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      isScrollControlled: true,
      backgroundColor: surface,
      builder: (sheetContext) => StatefulBuilder(
        builder: (sheetContext, update) => SizedBox(
          height: MediaQuery.sizeOf(sheetContext).height * .8,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              space32,
              space8,
              space32,
              space24,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Your packs',
                  style: TextStyle(
                    color: ink,
                    fontSize: 28,
                    letterSpacing: -.7,
                  ),
                ),
                const SizedBox(height: space8),
                const Text(
                  'Choose once. The patcher remembers.',
                  style: TextStyle(color: muted),
                ),
                const SizedBox(height: space16),
                TextField(
                  decoration: const InputDecoration(
                    prefixIcon: Icon(Icons.search),
                    hintText: 'Find a pack',
                    border: OutlineInputBorder(),
                  ),
                  onChanged: (text) => update(() => query = text.toLowerCase()),
                ),
                const SizedBox(height: space16),
                Expanded(
                  child: ListView(
                    children: [
                      for (final p in packs.where(
                        (p) =>
                            p['name'].toString().toLowerCase().contains(query),
                      ))
                        CheckboxListTile(
                          key: Key('pack-${p['id']}'),
                          contentPadding: EdgeInsets.zero,
                          tristate: !selections.containsKey(p['id']),
                          title: Text(p['name']),
                          subtitle: Text(
                            [
                              p['id'],
                              '${formatBytes(p['bytes'] ?? 0)} download',
                              if (!selections.containsKey(p['id']))
                                deferred.contains(p['id'])
                                    ? 'Decided later'
                                    : 'New: not decided',
                            ].join(' - '),
                          ),
                          value: selections[p['id']],
                          onChanged: (v) => update(() {
                            selections[p['id']] = v ?? false;
                            deferred.remove(p['id']);
                          }),
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: space16),
                Wrap(
                  spacing: space16,
                  runSpacing: space16,
                  children: [
                    FilledButton(
                      key: const Key('save-packs'),
                      onPressed: () {
                        Navigator.pop(sheetContext);
                        send('SetPolicy', {
                          'revision': s['policy_revision'],
                          'dlc': selections,
                          'deferred': deferred.toList(),
                        });
                      },
                      child: const Text('Save selections'),
                    ),
                    if (packs.any(
                      (p) => undecided(p) && !selections.containsKey(p['id']),
                    ))
                      TextButton(
                        key: const Key('defer-packs'),
                        onPressed: () {
                          Navigator.pop(sheetContext);
                          send('SetPolicy', {
                            'revision': s['policy_revision'],
                            'dlc': selections,
                            'deferred': {
                              ...deferred,
                              for (final p in packs)
                                if (undecided(p) &&
                                    !selections.containsKey(p['id']))
                                  p['id'] as String,
                            }.toList(),
                          });
                        },
                        child: const Text('Decide new packs later'),
                      ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The status beside the game name: an icon and a word, so it reads without colour.
class StatusLabel extends StatelessWidget {
  const StatusLabel({super.key, required this.status});
  final Map<String, dynamic> status;
  @override
  Widget build(BuildContext context) {
    final text = status['status']?.toString() ?? '';
    final play = status['play']?.toString();
    final (IconData icon, Color color) = switch (text) {
      'Playing' => (Icons.sports_esports_outlined, teal),
      'Ready to play' || 'Ready' => (Icons.check_circle_outline_rounded, teal),
      'Needs attention' => (Icons.error_outline_rounded, warning),
      'Checking' ||
      'Verifying' ||
      'Downloading' ||
      'Applying' ||
      'Starting' => (Icons.sync_rounded, muted),
      'Paused' || 'Stopped' => (Icons.pause_circle_outline_rounded, muted),
      _ => (
        play == 'starting'
            ? Icons.sync_rounded
            : Icons.radio_button_unchecked_rounded,
        muted,
      ),
    };
    return Row(
      key: const Key('status-label'),
      children: [
        Icon(icon, size: 14, color: color),
        const SizedBox(width: space4),
        Flexible(
          child: Text(
            text,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(color: muted, fontSize: 12),
          ),
        ),
      ],
    );
  }
}

/// The pack summary in the bottom slot; opens the pack list when the engine offers one.
class CollectionTile extends StatelessWidget {
  const CollectionTile({super.key, required this.summary, this.onTap});
  final String summary;
  final VoidCallback? onTap;
  @override
  Widget build(BuildContext context) => InkWell(
    key: const Key('packs-action'),
    borderRadius: BorderRadius.circular(radiusSmall + 4),
    onTap: onTap,
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: space8, horizontal: space8),
      child: Row(
        children: [
          Icon(
            Icons.grid_view_rounded,
            color: onTap == null ? muted : teal,
            size: 20,
          ),
          const SizedBox(width: space16),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Your packs',
                  style: TextStyle(color: ink, fontWeight: FontWeight.w600),
                ),
                Text(
                  summary,
                  key: const Key('collection-summary'),
                  style: const TextStyle(color: muted, fontSize: 12),
                ),
              ],
            ),
          ),
          if (onTap != null)
            const Icon(Icons.chevron_right_rounded, color: muted),
        ],
      ),
    ),
  );
}

class ProgressView extends StatelessWidget {
  const ProgressView({super.key, required this.core, this.controls = const []});
  final Core core;

  /// Pause, Resume and Stop, kept beside the progress they control.
  final List<Widget> controls;

  @override
  Widget build(BuildContext context) => Select<List<num>>(
    core: core,
    selector: (p) => [
      p['bytes_done'] as num? ?? 0,
      p['bytes_total'] as num? ?? 0,
      p['bytes_per_second'] as num? ?? 0,
      p['files_done'] as num? ?? 0,
      p['files_total'] as num? ?? 0,
    ],
    builder: (context, p) {
      final [done, total, rate, files, filesTotal] = p;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          LinearProgressIndicator(
            key: const Key('progress'),
            value: total > 0 ? (done / total).clamp(0, 1).toDouble() : null,
            minHeight: 6,
            borderRadius: BorderRadius.circular(radiusSmall),
            backgroundColor: mint,
            color: teal,
          ),
          const SizedBox(height: space8),
          Wrap(
            spacing: space16,
            runSpacing: space8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Text(
                [
                  if (filesTotal > 0) '$files of $filesTotal files',
                  if (total > 0)
                    '${formatBytes(done)} of ${formatBytes(total)}',
                  if (total > 0 && rate > 0) '${formatBytes(rate)}/s',
                ].join(' - ').ifEmpty('Starting'),
                style: const TextStyle(color: muted, fontSize: 12),
              ),
              ...controls,
            ],
          ),
        ],
      );
    },
  );
}

class BadgeLabel extends StatelessWidget {
  const BadgeLabel({super.key, required this.text});
  final String text;
  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: space8, vertical: space4),
    decoration: BoxDecoration(
      border: Border.all(color: line),
      borderRadius: BorderRadius.circular(radiusPill),
    ),
    child: Text(
      text,
      style: const TextStyle(
        color: muted,
        fontSize: 9,
        letterSpacing: 1,
        fontWeight: FontWeight.w600,
      ),
    ),
  );
}

String formatBytes(num value) {
  if (value >= 1073741824) {
    return '${(value / 1073741824).toStringAsFixed(1)} GB';
  }
  if (value >= 1048576) {
    return '${(value / 1048576).toStringAsFixed(1)} MB';
  }
  if (value >= 1024) {
    return '${(value / 1024).toStringAsFixed(0)} KB';
  }
  return '${value.toInt()} B';
}

const bandwidthChoices = [
  ('Unlimited', 0),
  ('1 MB/s', 1048576),
  ('5 MB/s', 5242880),
  ('20 MB/s', 20971520),
];

String languageName(String code) => switch (code) {
  'ENG_US' => 'English',
  'GER_DE' => 'Deutsch',
  'FRE_FR' => 'Français',
  'SPA_ES' => 'Español',
  'ITA_IT' => 'Italiano',
  'POR_BR' => 'Português (Brasil)',
  'DUT_NL' => 'Nederlands',
  'POL_PL' => 'Polski',
  'RUS_RU' => 'Русский',
  'CZE_CZ' => 'Čeština',
  'DAN_DK' => 'Dansk',
  'FIN_FI' => 'Suomi',
  'NOR_NO' => 'Norsk',
  'SWE_SE' => 'Svenska',
  'JPN_JP' => '日本語',
  'KOR_KR' => '한국어',
  'CHS_CN' => '简体中文',
  'CHT_CN' => '繁體中文',
  _ => code,
};

extension on String {
  String ifEmpty(String fallback) => isEmpty ? fallback : this;
}
