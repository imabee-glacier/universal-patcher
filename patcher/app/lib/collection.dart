import 'package:flutter/material.dart';

import 'main.dart' show ink, teal, paper, muted, line, formatBytes;

/// Colours for each kind of pack: one hue per kind, used as a band and tag.
const kindColors = {
  'expansion': Color(0xff176b5b),
  'game': Color(0xff5b4a8b),
  'stuff': Color(0xffb7791f),
  'kit': Color(0xffb04a6a),
  'free': Color(0xff697772),
};
const kindNames = {
  'expansion': 'Expansion',
  'game': 'Game pack',
  'stuff': 'Stuff pack',
  'kit': 'Kit',
  'free': 'Free',
};
const attention = Color(0xffb7791f);

bool needsAttention(Map<String, dynamic> p) =>
    p['status'] == 'incomplete' || p['status'] == 'changed';
bool isInstalled(Map<String, dynamic> p) =>
    p['status'] != 'not_installed' && p['status'] != '';

/// Every pack the game knows, with what is on this computer and what the user wants to keep.
class CollectionPage extends StatefulWidget {
  const CollectionPage({super.key, required this.status, required this.save});

  final Map<String, dynamic> status;

  /// Saves the decisions; returns null when accepted, or the reason it was not.
  final String? Function(Map<String, bool> dlc) save;

  @override
  State<CollectionPage> createState() => _CollectionPageState();
}

class _CollectionPageState extends State<CollectionPage> {
  late final List<Map<String, dynamic>> packs;
  late final Map<String, bool> choice;
  String query = '';
  String kind = 'all';
  String state = 'all';
  bool dirty = false;

  @override
  void initState() {
    super.initState();
    packs = List<Map<String, dynamic>>.from(widget.status['packs'] ?? []);
    choice = {for (final p in packs) p['id'] as String: p['selected'] == true};
    // Unanswered packs start dirty: saving confirms the proposal.
    dirty = packs.any((p) => isInstalled(p) && p['answered'] != true);
  }

  List<Map<String, dynamic>> get shown => packs.where((p) {
    final text = '${p['name']} ${p['id']}'.toLowerCase();
    if (query.isNotEmpty && !text.contains(query)) return false;
    if (kind != 'all' && p['kind'] != kind) return false;
    return switch (state) {
      'installed' => isInstalled(p),
      'not_installed' => p['status'] == 'not_installed',
      'attention' => needsAttention(p),
      _ => true,
    };
  }).toList();

  void issues(Map<String, dynamic> p) => showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(p['name']),
      content: SizedBox(
        width: 520,
        child: SelectionArea(
          child: ListView(
            shrinkWrap: true,
            children: [
              for (final issue in List<dynamic>.from(p['issues'] ?? []))
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 2),
                  child: Text(
                    issue.toString(),
                    style: const TextStyle(fontSize: 12, color: muted),
                  ),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Close'),
        ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    final installed = packs.where(isInstalled).length;
    final flagged = packs.where(needsAttention).length;
    final onDisk = packs.fold<num>(
      0,
      (sum, p) => sum + (p['installed_bytes'] as num? ?? 0),
    );
    final review = packs.any((p) => isInstalled(p) && p['answered'] != true);
    final list = shown;
    return Scaffold(
      backgroundColor: paper,
      appBar: AppBar(
        backgroundColor: paper,
        surfaceTintColor: paper,
        title: const Text('Your collection', style: TextStyle(color: ink)),
      ),
      body: LayoutBuilder(
        builder: (context, box) {
          final columns = (box.maxWidth / 260).floor().clamp(1, 5);
          return CustomScrollView(
            slivers: [
              SliverPadding(
                padding: const EdgeInsets.fromLTRB(24, 4, 24, 0),
                sliver: SliverList.list(
                  children: [
                    Wrap(
                      spacing: 10,
                      runSpacing: 10,
                      children: [
                        _Stat(
                          key: const Key('stat-installed'),
                          icon: Icons.check_circle_outline_rounded,
                          color: teal,
                          text: '$installed installed',
                        ),
                        _Stat(
                          icon: Icons.radio_button_unchecked,
                          color: muted,
                          text: '${packs.length - installed} not installed',
                        ),
                        if (flagged > 0)
                          _Stat(
                            key: const Key('stat-attention'),
                            icon: Icons.error_outline_rounded,
                            color: attention,
                            text: '$flagged need attention',
                          ),
                        if (onDisk > 0)
                          _Stat(
                            icon: Icons.storage_rounded,
                            color: muted,
                            text: '${formatBytes(onDisk)} on disk',
                          ),
                      ],
                    ),
                    if (review) ...[
                      const SizedBox(height: 16),
                      Container(
                        key: const Key('review-banner'),
                        padding: const EdgeInsets.all(16),
                        decoration: BoxDecoration(
                          color: const Color(0xffe8eee6),
                          borderRadius: BorderRadius.circular(14),
                        ),
                        child: const Row(
                          children: [
                            Icon(Icons.auto_awesome_outlined, color: teal),
                            SizedBox(width: 12),
                            Expanded(
                              child: Text(
                                'The packs installed on this computer are ticked. '
                                'Untick any you do not want to keep, then save.',
                              ),
                            ),
                          ],
                        ),
                      ),
                    ],
                    const SizedBox(height: 16),
                    TextField(
                      key: const Key('pack-search'),
                      decoration: InputDecoration(
                        prefixIcon: const Icon(Icons.search),
                        hintText: 'Find a pack by name or id',
                        filled: true,
                        fillColor: Colors.white,
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(14),
                          borderSide: const BorderSide(color: line),
                        ),
                      ),
                      onChanged: (t) => setState(() => query = t.toLowerCase()),
                    ),
                    const SizedBox(height: 12),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        for (final (value, label) in [
                          ('all', 'All kinds'),
                          ('expansion', 'Expansions'),
                          ('game', 'Game packs'),
                          ('stuff', 'Stuff packs'),
                          ('kit', 'Kits'),
                          ('free', 'Free'),
                        ])
                          ChoiceChip(
                            key: Key('kind-$value'),
                            label: Text(label),
                            selected: kind == value,
                            onSelected: (_) => setState(() => kind = value),
                          ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        for (final (value, label) in [
                          ('all', 'Everything'),
                          ('installed', 'Installed'),
                          ('not_installed', 'Not installed'),
                          ('attention', 'Needs attention'),
                        ])
                          ChoiceChip(
                            key: Key('state-$value'),
                            label: Text(label),
                            selected: state == value,
                            onSelected: (_) => setState(() => state = value),
                          ),
                      ],
                    ),
                    const SizedBox(height: 16),
                    if (list.isEmpty)
                      const Padding(
                        padding: EdgeInsets.all(32),
                        child: Center(
                          child: Text(
                            'No packs match. Try another name or filter.',
                            style: TextStyle(color: muted),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
              SliverPadding(
                padding: const EdgeInsets.fromLTRB(24, 0, 24, 110),
                sliver: SliverGrid.builder(
                  gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                    crossAxisCount: columns,
                    mainAxisSpacing: 14,
                    crossAxisSpacing: 14,
                    // Tall enough for two-line names at larger text sizes.
                    mainAxisExtent: 196,
                  ),
                  itemCount: list.length,
                  itemBuilder: (context, i) {
                    final p = list[i];
                    final id = p['id'] as String;
                    return PackCard(
                      key: Key('pack-$id'),
                      pack: p,
                      selected: choice[id] == true,
                      onToggle: () => setState(() {
                        choice[id] = !(choice[id] ?? false);
                        dirty = true;
                      }),
                      onIssues: needsAttention(p) ? () => issues(p) : null,
                    );
                  },
                ),
              ),
            ],
          );
        },
      ),
      bottomSheet: Container(
        color: paper,
        padding: const EdgeInsets.fromLTRB(24, 12, 24, 20),
        child: Row(
          children: [
            Expanded(
              child: Text(
                '${choice.values.where((v) => v).length} of ${packs.length} packs in your collection',
                style: const TextStyle(color: muted),
              ),
            ),
            FilledButton.icon(
              key: const Key('save-packs'),
              onPressed: dirty
                  ? () {
                      final error = widget.save(Map.of(choice));
                      if (error == null) {
                        Navigator.pop(context);
                      } else {
                        ScaffoldMessenger.of(context)
                            .showSnackBar(SnackBar(content: Text(error)));
                      }
                    }
                  : null,
              icon: const Icon(Icons.check_rounded),
              label: Text(review ? 'Confirm my packs' : 'Save choices'),
            ),
          ],
        ),
      ),
    );
  }
}

class _Stat extends StatelessWidget {
  const _Stat({
    super.key,
    required this.icon,
    required this.color,
    required this.text,
  });
  final IconData icon;
  final Color color;
  final String text;
  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
    decoration: BoxDecoration(
      color: Colors.white,
      borderRadius: BorderRadius.circular(20),
      border: Border.all(color: line),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(icon, size: 16, color: color),
        const SizedBox(width: 6),
        Text(text, style: const TextStyle(color: ink, fontSize: 13)),
      ],
    ),
  );
}

/// One pack: its kind, name, what is on this computer, and whether it is kept.
class PackCard extends StatelessWidget {
  const PackCard({
    super.key,
    required this.pack,
    required this.selected,
    required this.onToggle,
    this.onIssues,
  });
  final Map<String, dynamic> pack;
  final bool selected;
  final VoidCallback onToggle;
  final VoidCallback? onIssues;

  @override
  Widget build(BuildContext context) {
    final kind = pack['kind']?.toString() ?? 'kit';
    final color = kindColors[kind] ?? muted;
    final status = pack['status']?.toString() ?? '';
    final (IconData icon, Color tone, String label) = switch (status) {
      'installed' => (Icons.check_circle_rounded, teal, 'Installed'),
      'incomplete' => (Icons.error_outline_rounded, attention, 'Incomplete'),
      'changed' => (Icons.change_circle_outlined, attention, 'Changed'),
      'not_installed' => (Icons.radio_button_unchecked, muted, 'Not installed'),
      _ => (Icons.cloud_outlined, muted, 'Available'),
    };
    final size = status == ''
        ? '${formatBytes(pack['bytes'] ?? 0)} download'
        : (pack['installed_bytes'] as num? ?? 0) > 0
        ? formatBytes(pack['installed_bytes'])
        : '';
    return Material(
      color: Colors.white,
      borderRadius: BorderRadius.circular(16),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: onToggle,
        child: Container(
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(16),
            border: Border.all(
              color: selected ? teal : line,
              width: selected ? 2 : 1,
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Container(height: 6, color: color),
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(14, 10, 8, 12),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: Text(
                              '${kindNames[kind] ?? kind} · ${pack['id']}'
                                  .toUpperCase(),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                color: color,
                                fontSize: 10,
                                fontWeight: FontWeight.w700,
                                letterSpacing: 1,
                              ),
                            ),
                          ),
                          Checkbox(
                            value: selected,
                            onChanged: (_) => onToggle(),
                            visualDensity: VisualDensity.compact,
                            materialTapTargetSize:
                                MaterialTapTargetSize.shrinkWrap,
                          ),
                        ],
                      ),
                      Flexible(
                        child: Text(
                          pack['name']?.toString() ?? '',
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                            color: ink,
                            fontSize: 17,
                            fontWeight: FontWeight.w600,
                            height: 1.2,
                          ),
                        ),
                      ),
                      const SizedBox(height: 8),
                      Row(
                        children: [
                          Icon(icon, size: 16, color: tone),
                          const SizedBox(width: 6),
                          Flexible(
                            child: Text(
                              label,
                              style: TextStyle(color: tone, fontSize: 13),
                            ),
                          ),
                          if (onIssues != null)
                            TextButton(
                              key: Key('issues-${pack['id']}'),
                              onPressed: onIssues,
                              style: TextButton.styleFrom(
                                visualDensity: VisualDensity.compact,
                                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 6,
                                ),
                                minimumSize: Size.zero,
                                foregroundColor: tone,
                              ),
                              child: const Text('Why?'),
                            ),
                          const SizedBox(width: 6),
                          Expanded(
                            child: Text(
                              size,
                              textAlign: TextAlign.end,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(
                                color: muted,
                                fontSize: 12,
                              ),
                            ),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
