#!/usr/bin/env bash
#
# Manual test of MTN MoMo's sandbox Collections API, to capture the callback
# MTN sends to X-Callback-Url. Not part of vpay; run it by hand.
#
#   ./test.sh setup               create an API user and API key, mint a token
#   ./test.sh pay N [ANSWER]      request-to-pay number N, then poll until
#                                 SUCCESSFUL. ANSWER is what you set the
#                                 inspector to return (e.g. 200 or 500); it is
#                                 only recorded in the evidence.
#   ./test.sh status N            read payment N's status once more
#   ./test.sh reset               delete the cached credentials in .state/
#
# Environment:
#   MTN_SUBSCRIPTION_KEY   Collections subscription key (every command)
#   INSPECTOR_HOST         callback host, e.g. webhook.site (setup)
#   INSPECTOR_URL          full callback URL on that host (pay)
#
# The subscription key, API key and access token are never printed and never
# written to ./evidence/. Because `pay` runs in a different process from
# `setup`, the API key and token are cached in .state/credentials.json
# (git-ignored, owner-only); `reset` deletes it.

set -euo pipefail
umask 077

readonly BASE_URL="https://sandbox.momodeveloper.mtn.com"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly EVIDENCE_DIR="$SCRIPT_DIR/evidence"
readonly STATE_DIR="$SCRIPT_DIR/.state"
readonly STATE_FILE="$STATE_DIR/credentials.json"
readonly POLL_INTERVAL_S=5
readonly POLL_TIMEOUT_S=120
readonly PAYER_MSISDN="46733123454"

SECRETS=()
TMP=""
SAVE_ERRORS_TO=""
LAST_STATUS=""
LAST_BODY=""

API_USER=""
API_KEY=""
CALLBACK_HOST=""
ACCESS_TOKEN=""
TOKEN_EXPIRES_AT=0

PAYMENT_DIR=""
PAY_N=""
ANSWER=""
REF=""
SENT_AT=""
ACCEPTED_AT=""
BODY_EMPTY="null"
OUTCOME=""
SUCCEEDED_AT=""
SUCCEEDED_AT_LOCAL=""

die() {
  echo "ERROR: $*" >&2
  exit 1
}
step() { printf '\n==> %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
now_utc() { date -u +%Y-%m-%dT%H:%M:%SZ; }
now_local() { date '+%Y-%m-%d %H:%M:%S %z'; }

usage() {
  cat <<'EOF'
Usage:
  ./test.sh setup               create an API user and API key, mint a token
  ./test.sh pay N [ANSWER]      request-to-pay number N, then poll its status
  ./test.sh status N            read payment N's status once more
  ./test.sh reset               delete the cached credentials in .state/

Environment: MTN_SUBSCRIPTION_KEY, INSPECTOR_HOST (setup), INSPECTOR_URL (pay)
EOF
}

# --- redaction ------------------------------------------------------------

add_secret() {
  if [[ -n ${1:-} ]]; then SECRETS+=("$1"); fi
}

# Replaces every known secret value on stdin with [REDACTED].
redact() {
  local text secret
  text=$(cat)
  for secret in ${SECRETS[@]+"${SECRETS[@]}"}; do
    text=${text//"$secret"/[REDACTED]}
  done
  printf '%s\n' "$text"
}

# --- helpers --------------------------------------------------------------

# jq.exe on Windows can end lines with CRLF, and a stray \r would corrupt
# every value read into a variable.
jqr() { jq -r "$@" | tr -d '\r'; }

require_tools() {
  local tool missing=()
  for tool in curl jq od base64 date mktemp; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if ((${#missing[@]})); then
    die "missing tools: ${missing[*]} (jq on Windows: winget install jqlang.jq, then reopen Git Bash)"
  fi
}

require_env() {
  local name
  for name in "$@"; do
    [[ -n ${!name:-} ]] || die "environment variable $name is not set"
  done
}

# A control character or space in a header value makes MTN's front server
# answer an HTML 400 ("The request is badly formed") that names nothing.
check_subscription_key() {
  local key=$MTN_SUBSCRIPTION_KEY
  # A Collections Primary key is exactly 32 hex characters. Anything else
  # means extra text was copied with it, and MTN answers an opaque 400 or 401.
  if [[ $key =~ [^0-9a-fA-F] || ${#key} -ne 32 ]]; then
    die "MTN_SUBSCRIPTION_KEY is ${#key} characters; a Collections Primary key is exactly 32 characters, 0-9 and a-f only. Copy the key again, selecting nothing but the key."
  fi
}

check_inspector_url() {
  if [[ $INSPECTOR_URL == *'#'* ]]; then
    die "INSPECTOR_URL contains '#', so it is webhook.site's viewing page and nothing after '#' reaches the server. Use https://webhook.site/<your-id> instead."
  fi
  if [[ $INSPECTOR_URL != https://* ]]; then
    die "INSPECTOR_URL must start with https://"
  fi
}

uuid_v4() {
  local h variant
  h=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
  variant=$(((0x${h:16:2} & 0x3f) | 0x80))
  printf '%s-%s-4%s-%02x%s-%s\n' \
    "${h:0:8}" "${h:8:4}" "${h:13:3}" "$variant" "${h:18:2}" "${h:20:12}"
}

host_of() {
  local h=${1#*://}
  h=${h%%/*}
  h=${h%%\?*}
  h=${h%%#*}
  h=${h##*@}
  h=${h%%:*}
  printf '%s\n' "${h,,}"
}

# call METHOD PATH EXPECTED_STATUS BODY [HEADER...]
#
# Sends the subscription key on every request. Headers are passed through a
# file (curl -H @file) so no credential appears on curl's command line.
# Sets LAST_STATUS and LAST_BODY; exits on any status but EXPECTED_STATUS.
call() {
  local method=$1 path=$2 expected=$3 body=$4
  shift 4
  local headers="$TMP/request-headers" h
  {
    printf 'Ocp-Apim-Subscription-Key: %s\n' "$MTN_SUBSCRIPTION_KEY"
    for h in "$@"; do printf '%s\n' "$h"; done
  } >"$headers"

  local args=(-sS --max-time 30 -X "$method" -H "@$headers"
    -D "$TMP/response-headers" -o "$TMP/response-body" -w '%{http_code}')
  if [[ -n $body ]]; then
    args+=(--data-binary "$body")
  elif [[ $method == POST ]]; then
    # A bodyless POST with no Content-Length is answered 411 by MTN's gateway.
    args+=(-H 'Content-Length: 0')
  fi

  : >"$TMP/response-headers"
  : >"$TMP/response-body"
  if ! LAST_STATUS=$(curl "${args[@]}" "$BASE_URL$path" 2>"$TMP/curl-error"); then
    rm -f "$headers"
    die "$method $path got no HTTP response: $(redact <"$TMP/curl-error")"
  fi
  rm -f "$headers"
  LAST_BODY=$(tr -d '\r' <"$TMP/response-body")

  if [[ $LAST_STATUS != "$expected" ]]; then
    local report
    report=$(printf '%s %s returned HTTP %s, expected %s.\nResponse body:\n%s' \
      "$method" "$path" "$LAST_STATUS" "$expected" "${LAST_BODY:-<empty>}" | redact)
    if [[ -n $SAVE_ERRORS_TO ]]; then printf '%s\n' "$report" >"$SAVE_ERRORS_TO"; fi
    die "$report"
  fi
}

# --- credentials ----------------------------------------------------------

save_state() {
  mkdir -p "$STATE_DIR"
  API_USER=$API_USER API_KEY=$API_KEY CALLBACK_HOST=$CALLBACK_HOST \
    ACCESS_TOKEN=$ACCESS_TOKEN TOKEN_EXPIRES_AT=$TOKEN_EXPIRES_AT \
    jq -n '{
      api_user: env.API_USER,
      api_key: env.API_KEY,
      provider_callback_host: env.CALLBACK_HOST,
      access_token: env.ACCESS_TOKEN,
      token_expires_at: (env.TOKEN_EXPIRES_AT | tonumber)
    }' >"$STATE_FILE.tmp"
  mv "$STATE_FILE.tmp" "$STATE_FILE"
  chmod 600 "$STATE_FILE"
}

load_state() {
  [[ -f $STATE_FILE ]] || die "no credentials yet: run ./test.sh setup first"
  API_USER=$(jqr '.api_user' <"$STATE_FILE")
  API_KEY=$(jqr '.api_key' <"$STATE_FILE")
  CALLBACK_HOST=$(jqr '.provider_callback_host' <"$STATE_FILE")
  ACCESS_TOKEN=$(jqr '.access_token' <"$STATE_FILE")
  TOKEN_EXPIRES_AT=$(jqr '.token_expires_at' <"$STATE_FILE")
  add_secret "$API_KEY"
  add_secret "$ACCESS_TOKEN"
}

mint_token() {
  local basic expires_in
  basic=$(printf '%s:%s' "$API_USER" "$API_KEY" | base64 | tr -d '\r\n')
  add_secret "$basic"
  # JSON, not form-encoded: MTN rejects the form spelling of this grant.
  call POST /collection/token/ 200 '{"grant_type":"client_credentials"}' \
    "Authorization: Basic $basic" "Content-Type: application/json"
  ACCESS_TOKEN=$(printf '%s' "$LAST_BODY" | jqr '.access_token // empty') ||
    die "the token response was not JSON"
  [[ -n $ACCESS_TOKEN ]] || die "the token response had no access_token"
  add_secret "$ACCESS_TOKEN"
  expires_in=$(printf '%s' "$LAST_BODY" | jqr '.expires_in // 3600')
  TOKEN_EXPIRES_AT=$(($(date +%s) + expires_in))
  info "HTTP 200: access token received (expires in ${expires_in}s, not shown)"
}

ensure_token() {
  if (($(date +%s) >= TOKEN_EXPIRES_AT - 60)); then
    step "The cached access token has expired; minting a new one"
    mint_token
    save_state
  fi
}

# --- commands -------------------------------------------------------------

cmd_setup() {
  require_env MTN_SUBSCRIPTION_KEY INSPECTOR_HOST
  check_subscription_key
  add_secret "$MTN_SUBSCRIPTION_KEY"
  [[ ! -f $STATE_FILE ]] ||
    die "credentials already exist in $STATE_FILE. Run ./test.sh reset first to create a new API user."
  CALLBACK_HOST=$INSPECTOR_HOST
  if [[ $CALLBACK_HOST == *"://"* || $CALLBACK_HOST == */* ]]; then
    die "INSPECTOR_HOST must be a bare host such as webhook.site, not a URL"
  fi
  mkdir -p "$EVIDENCE_DIR"
  local started
  started=$(now_utc)

  step "1/3  Creating a sandbox API user"
  API_USER=$(uuid_v4)
  info "X-Reference-Id (API user): $API_USER"
  info "providerCallbackHost:      $CALLBACK_HOST"
  call POST /v1_0/apiuser 201 \
    "$(jqr -cn --arg h "$CALLBACK_HOST" '{providerCallbackHost: $h}')" \
    "X-Reference-Id: $API_USER" "Content-Type: application/json"
  info "HTTP 201: API user created"

  step "2/3  Creating an API key for that user"
  call POST "/v1_0/apiuser/$API_USER/apikey" 201 ""
  API_KEY=$(printf '%s' "$LAST_BODY" | jqr '.apiKey // empty') ||
    die "the API key response was not JSON"
  [[ -n $API_KEY ]] || die "the API key response had no apiKey field"
  add_secret "$API_KEY"
  info "HTTP 201: API key received (not shown)"

  step "3/3  Minting a Collections access token"
  mint_token
  save_state

  jq -n --arg at "$started" --arg user "$API_USER" --arg host "$CALLBACK_HOST" '{
      setup_at: $at,
      api_user_reference_id: $user,
      provider_callback_host: $host,
      calls: [
        {call: "POST /v1_0/apiuser", http_status: 201},
        {call: "POST /v1_0/apiuser/{id}/apikey", http_status: 201},
        {call: "POST /collection/token/", http_status: 200}
      ]
    }' | redact >"$EVIDENCE_DIR/setup.json"

  step "Setup done"
  info "Evidence: evidence/setup.json"
  info "Next: set the inspector's answer, then run ./test.sh pay 1 200"
}

make_payment() {
  local body
  REF=$(uuid_v4)
  body=$(jqr -cn --arg ref "$REF" --arg msisdn "$PAYER_MSISDN" '{
      amount: "5", currency: "EUR", externalId: $ref,
      payer: {partyIdType: "MSISDN", partyId: $msisdn},
      payerMessage: "vpay human test", payeeNote: "vpay human test"
    }')

  step "Payment $PAY_N: POST /collection/v1_0/requesttopay"
  info "X-Reference-Id: $REF (also the externalId)"
  info "X-Callback-Url: $INSPECTOR_URL"
  info "Inspector set to answer: $ANSWER"

  {
    echo "POST $BASE_URL/collection/v1_0/requesttopay"
    echo "Authorization: Bearer [REDACTED]"
    echo "Ocp-Apim-Subscription-Key: [REDACTED]"
    echo "X-Reference-Id: $REF"
    echo "X-Target-Environment: sandbox"
    echo "X-Callback-Url: $INSPECTOR_URL"
    echo "Content-Type: application/json"
    echo
    printf '%s\n' "$body"
  } | redact >"$PAYMENT_DIR/request.txt"

  SENT_AT=$(now_utc)
  call POST /collection/v1_0/requesttopay 202 "$body" \
    "Authorization: Bearer $ACCESS_TOKEN" \
    "X-Reference-Id: $REF" \
    "X-Target-Environment: sandbox" \
    "X-Callback-Url: $INSPECTOR_URL" \
    "Content-Type: application/json"
  ACCEPTED_AT=$(now_utc)
  tr -d '\r' <"$TMP/response-headers" | redact >"$PAYMENT_DIR/response-202-headers.txt"

  if [[ -z $LAST_BODY ]]; then
    BODY_EMPTY=true
    info "HTTP 202 with an empty body at $ACCEPTED_AT"
  else
    BODY_EMPTY=false
    printf '%s\n' "$LAST_BODY" | redact >"$PAYMENT_DIR/response-202-body.txt"
    info "HTTP 202 at $ACCEPTED_AT, but the body was NOT empty (saved to response-202-body.txt)"
  fi
}

poll_payment() {
  local status at start=$SECONDS
  step "Polling GET /collection/v1_0/requesttopay/$REF every ${POLL_INTERVAL_S}s (up to ${POLL_TIMEOUT_S}s)"
  while :; do
    call GET "/collection/v1_0/requesttopay/$REF" 200 "" \
      "Authorization: Bearer $ACCESS_TOKEN" "X-Target-Environment: sandbox"
    at=$(now_utc)
    status=$(printf '%s' "$LAST_BODY" | jqr '.status // "NO_STATUS"' 2>/dev/null) ||
      status="UNPARSEABLE"
    printf '%s %s\n' "$at" "$status" >>"$PAYMENT_DIR/polls.log"
    printf '%s\n' "$LAST_BODY" | redact >"$PAYMENT_DIR/final-status-response.json"
    info "$at  $status"

    if [[ $status == SUCCESSFUL ]]; then
      OUTCOME=SUCCESSFUL
      SUCCEEDED_AT=$at
      SUCCEEDED_AT_LOCAL=$(now_local)
      break
    fi
    # Sandbox status moves CREATED, then SUCCESSFUL. CREATED is not final.
    if [[ $status != PENDING && $status != CREATED ]]; then
      OUTCOME=$status
      break
    fi
    if ((SECONDS - start >= POLL_TIMEOUT_S)); then
      OUTCOME=GAVE_UP
      break
    fi
    sleep "$POLL_INTERVAL_S"
  done

  write_payment_json
  case $OUTCOME in
    SUCCESSFUL)
      step "Payment $PAY_N SUCCESSFUL at $SUCCEEDED_AT (local: $SUCCEEDED_AT_LOCAL)"
      info "Evidence: evidence/payment-$PAY_N/"
      info "Now check the inspector and fill in evidence-template.md."
      ;;
    GAVE_UP)
      die "payment $PAY_N was still ${status:-unknown} after ${POLL_TIMEOUT_S}s. Re-check later with ./test.sh status $PAY_N"
      ;;
    *)
      die "payment $PAY_N ended with status $OUTCOME; see evidence/payment-$PAY_N/final-status-response.json"
      ;;
  esac
}

write_payment_json() {
  [[ -n $PAYMENT_DIR ]] || return 0
  local polls="" final=""
  [[ -f $PAYMENT_DIR/polls.log ]] && polls=$(cat "$PAYMENT_DIR/polls.log")
  [[ -f $PAYMENT_DIR/final-status-response.json ]] &&
    final=$(cat "$PAYMENT_DIR/final-status-response.json")

  jq -n \
    --argjson payment "$PAY_N" \
    --arg ref "$REF" \
    --arg callback_url "$INSPECTOR_URL" \
    --arg answer "$ANSWER" \
    --arg sent "$SENT_AT" \
    --arg accepted "$ACCEPTED_AT" \
    --argjson body_empty "$BODY_EMPTY" \
    --arg outcome "${OUTCOME:-ERROR}" \
    --arg succeeded "$SUCCEEDED_AT" \
    --arg succeeded_local "$SUCCEEDED_AT_LOCAL" \
    --arg polls "$polls" \
    --arg final "$final" '
    def nullable: if . == "" then null else . end;
    {
      payment: $payment,
      x_reference_id: ($ref | nullable),
      external_id: ($ref | nullable),
      target_environment: "sandbox",
      callback_url: $callback_url,
      inspector_configured_answer: $answer,
      request_sent_at: ($sent | nullable),
      accepted_at: ($accepted | nullable),
      accepted_body_empty: $body_empty,
      polls: ($polls | split("\n") | map(select(length > 0) | split(" ")
               | {at: .[0], status: .[1]})),
      outcome: $outcome,
      succeeded_at: ($succeeded | nullable),
      succeeded_at_local: ($succeeded_local | nullable),
      final_status_response: (if $final == "" then null
                              else ($final | try fromjson catch .) end)
    }' | redact >"$PAYMENT_DIR/payment.json"
}

cmd_pay() {
  PAY_N=${1:-}
  ANSWER=${2:-unspecified}
  [[ $PAY_N =~ ^[1-9][0-9]*$ ]] || die "usage: ./test.sh pay N [INSPECTOR_ANSWER], e.g. ./test.sh pay 1 200"
  require_env MTN_SUBSCRIPTION_KEY INSPECTOR_URL
  check_subscription_key
  check_inspector_url
  add_secret "$MTN_SUBSCRIPTION_KEY"
  load_state

  local url_host
  url_host=$(host_of "$INSPECTOR_URL")
  [[ $url_host == "${CALLBACK_HOST,,}" ]] ||
    die "INSPECTOR_URL's host ($url_host) is not the providerCallbackHost registered at setup ($CALLBACK_HOST). MTN requires them to match."

  local dir="$EVIDENCE_DIR/payment-$PAY_N"
  [[ ! -e $dir ]] || die "evidence/payment-$PAY_N already exists. Move it away to re-run payment $PAY_N."
  mkdir -p "$dir"
  PAYMENT_DIR=$dir
  SAVE_ERRORS_TO="$dir/error.txt"
  : >"$dir/polls.log"

  ensure_token
  make_payment
  poll_payment
}

cmd_status() {
  local n=${1:-} dir status
  [[ $n =~ ^[1-9][0-9]*$ ]] || die "usage: ./test.sh status N"
  dir="$EVIDENCE_DIR/payment-$n"
  [[ -f $dir/payment.json ]] || die "no evidence/payment-$n/payment.json"
  require_env MTN_SUBSCRIPTION_KEY
  check_subscription_key
  add_secret "$MTN_SUBSCRIPTION_KEY"
  load_state
  ensure_token

  REF=$(jqr '.x_reference_id // empty' <"$dir/payment.json")
  [[ -n $REF ]] || die "payment $n has no X-Reference-Id recorded (its request never got a 202)"
  step "GET /collection/v1_0/requesttopay/$REF"
  SAVE_ERRORS_TO="$dir/error-status.txt"
  call GET "/collection/v1_0/requesttopay/$REF" 200 "" \
    "Authorization: Bearer $ACCESS_TOKEN" "X-Target-Environment: sandbox"
  local at file
  at=$(now_utc)
  status=$(printf '%s' "$LAST_BODY" | jqr '.status // "NO_STATUS"' 2>/dev/null) || status="UNPARSEABLE"
  file="status-$(date -u +%Y%m%dT%H%M%SZ).json"
  printf '%s\n' "$LAST_BODY" | redact >"$dir/$file"
  info "$at  $status (saved to evidence/payment-$n/$file)"
}

cmd_reset() {
  rm -rf "$STATE_DIR"
  echo "Deleted .state/ (cached API key and token). evidence/ was not touched."
}

on_exit() {
  # A payment that stopped early still records its reference and timestamps.
  if [[ -n $PAYMENT_DIR && ! -f $PAYMENT_DIR/payment.json ]]; then
    OUTCOME=${OUTCOME:-ERROR}
    write_payment_json 2>/dev/null || true
  fi
  if [[ -n $TMP ]]; then rm -rf "$TMP"; fi
}

main() {
  local cmd=${1:-help}
  shift || true
  case $cmd in
    help | -h | --help)
      usage
      return 0
      ;;
    reset)
      cmd_reset
      return 0
      ;;
    setup | pay | status) ;;
    *)
      usage >&2
      exit 2
      ;;
  esac

  require_tools
  TMP=$(mktemp -d)
  # Native Windows curl/jq cannot open MSYS paths such as /tmp/...
  if command -v cygpath >/dev/null 2>&1; then TMP=$(cygpath -m "$TMP"); fi
  trap on_exit EXIT

  case $cmd in
    setup) cmd_setup "$@" ;;
    pay) cmd_pay "$@" ;;
    status) cmd_status "$@" ;;
  esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
