import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

RailSpec _push({
  String code = 'a_push_rail',
  List<RailField> fields = const [
    RailField(
      name: 'msisdn',
      kind: RailFieldKindPhone(
        region: 'CM',
        phoneType: RailFieldPhoneType.mobile,
      ),
      required: true,
      labelKey: 'msisdn.label',
    ),
  ],
}) => RailSpec(
  code: code,
  flow: RailFlow.push,
  labelKey: 'rail.$code',
  displayName: null,
  fields: fields,
);

RailSpec _redirect({String code = 'a_redirect_rail'}) => RailSpec(
  code: code,
  flow: RailFlow.redirect,
  labelKey: 'rail.$code',
  displayName: null,
  fields: const [],
);

void main() {
  group('railChoices — structural, never per-code (D9)', () {
    test('a push rail with only phone fields is supported', () {
      final RailChoices choices = railChoices([_push()]);
      expect(choices.supported, hasLength(1));
      expect(choices.supported.single.code, 'a_push_rail');
      expect(choices.unsupported, isEmpty);
    });

    test('a redirect rail with no fields is supported', () {
      final RailChoices choices = railChoices([_redirect()]);
      expect(choices.supported, hasLength(1));
      expect(choices.unsupported, isEmpty);
    });

    test('a rail with an unknown flow is unsupported, with its code named', () {
      final RailSpec spec = RailSpec(
        code: 'a_future_rail',
        flow: RailFlow.unknown,
        labelKey: 'rail.a_future_rail',
        displayName: null,
        fields: const [],
      );
      final RailChoices choices = railChoices([spec]);
      expect(choices.supported, isEmpty);
      expect(choices.unsupported, hasLength(1));
      expect(choices.unsupported.single.code, 'a_future_rail');
      expect(
        choices.unsupported.single.reason,
        UnsupportedRailReason.unknownFlow,
      );
    });

    test('a push rail whose field type this SDK does not recognise is '
        'unsupported (D9) — the structural mechanism that also keeps a card '
        'field out, without a single literal "card" anywhere', () {
      final RailSpec spec = _push(
        code: 'a_card_shaped_rail',
        fields: const [
          RailField(
            name: 'pan',
            kind: RailFieldKindUnknown('card'),
            required: true,
            labelKey: 'card.label',
          ),
        ],
      );
      final RailChoices choices = railChoices([spec]);
      expect(choices.supported, isEmpty);
      expect(choices.unsupported, hasLength(1));
      expect(choices.unsupported.single.code, 'a_card_shaped_rail');
      expect(
        choices.unsupported.single.reason,
        UnsupportedRailReason.unrenderableField,
      );
    });

    test('a rail this package has literally never heard of still renders from '
        'its own spec, purely because its flow and fields are known', () {
      // The point of #186/#189: nothing here is keyed by a code the SDK
      // was shipped knowing about.
      final RailSpec neverHeardOf = _push(code: 'zzz_totally_unknown_rail');
      final RailChoices choices = railChoices([neverHeardOf]);
      expect(choices.supported.single.code, 'zzz_totally_unknown_rail');
    });

    test('allowedMethods can only narrow, never add a rail the session did not offer', () {
      final RailChoices choices = railChoices(
        [_push(code: 'mtn_momo')],
        allowedMethods: ['mtn_momo', 'a_rail_the_session_never_offered'],
      );
      expect(choices.supported, hasLength(1));
      expect(choices.supported.single.code, 'mtn_momo');
    });

    test('a rail the session offers but the operator excludes lands in unsupported', () {
      final RailChoices choices = railChoices(
        [_push(code: 'mtn_momo'), _redirect(code: 'orange_money')],
        allowedMethods: ['mtn_momo'],
      );
      expect(choices.supported.map((r) => r.code), ['mtn_momo']);
      expect(choices.unsupported.map((u) => u.code), ['orange_money']);
    });

    test('order is preserved: supported rails come out in the session\'s own order', () {
      final RailChoices choices = railChoices([
        _push(code: 'first'),
        _redirect(code: 'second'),
        _push(code: 'third'),
      ]);
      expect(choices.supported.map((r) => r.code).toList(), [
        'first',
        'second',
        'third',
      ]);
    });
  });
}
