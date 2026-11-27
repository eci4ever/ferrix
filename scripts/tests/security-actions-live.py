#!/usr/bin/env python3
"""Exercise real Security actions using disposable accounts/schema/API, never live users."""
import base64
import hashlib
import hmac
import importlib.util
from pathlib import Path
import secrets
import struct
import time
import uuid

spec = importlib.util.spec_from_file_location('live', Path(__file__).with_name('resource-permissions-live.py'))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
live.SCHEMA = 'security_actions_' + uuid.uuid4().hex


def otp(secret, step):
    key = base64.b32decode(secret + '='*((8-len(secret)%8)%8))
    digest = hmac.new(key, struct.pack('>Q', step), hashlib.sha1).digest()
    offset = digest[-1] & 15
    return f'{(struct.unpack(">I", digest[offset:offset+4])[0] & 0x7fffffff)%1000000:06d}'


def scenario(origin):
    owner, first, second, third, unrelated = (live.Client(origin) for _ in range(5))
    password = secrets.token_urlsafe(32)
    owner.request('POST', '/auth/setup', {'name': 'Disposable audit owner', 'email': 'owner@example.test', 'password': password})
    for client, name in [(first, 'audit'), (unrelated, 'unrelated')]:
        invitation = owner.request('POST', '/auth/invitations', {'email': name+'@example.test', 'role': 'member'})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': name+'@example.test', 'password': password, 'token': invitation['token']})
    def sign_in(client, value=password):
        return client.request('POST', '/auth/sign-in', {'email': 'audit@example.test', 'password': value})
    sign_in(second); sign_in(third)
    identity = first.request('GET', '/auth/session'); session = identity['session_id']
    assert len(first.request('GET', '/auth/sessions')) == 3
    # Age only the disposable session; this is a negative fresh-auth boundary test.
    live.sql(f"UPDATE {live.SCHEMA}.auth_sessions SET authenticated_at=now()-interval '16 minutes' WHERE id='{session}'")
    error = first.request('POST', '/auth/sessions/revoke-others', {}, expected=401)
    assert 'verify your password' in error['error']
    second.request('GET', '/auth/session'); third.request('GET', '/auth/session')
    first.request('POST', '/auth/reauthenticate', {'password': 'incorrect-audit-password'}, expected=401)
    first.request('POST', '/auth/reauthenticate', {'password': password}, expected=204)
    first.request('POST', '/auth/sessions/revoke-others', {}, expected=204)
    remaining = first.request('GET', '/auth/sessions')
    assert len(remaining)==1 and remaining[0]['id']==session and remaining[0]['current']
    second.request('GET', '/auth/session', expected=401); third.request('GET', '/auth/session', expected=401)
    unrelated.request('GET', '/auth/session')
    first.request('POST', '/auth/sessions/revoke-others', {}, expected=204)
    print('PASS: stale verification rejects revoke-others without deleting sessions; reauthentication permits it; current/unrelated sessions survive, other cookies get 401; repeated revoke is safe.', flush=True)
    sign_in(second)
    target = second.request('GET', '/auth/session')['session_id']
    first.request('POST', f'/auth/sessions/{target}/revoke', {}, expected=204)
    second.request('GET', '/auth/session', expected=401)
    first.request('POST', f'/auth/sessions/{session}/revoke', {}, expected=422)
    foreign = unrelated.request('GET', '/auth/session')['session_id']
    first.request('POST', f'/auth/sessions/{foreign}/revoke', {}, expected=204)
    unrelated.request('GET', '/auth/session')
    print('PASS: individual revoke invalidates its cookie; current-session revoke rejected; another user session cannot be revoked by ID substitution.', flush=True)
    sign_in(second)
    new_password = secrets.token_urlsafe(32)
    first.request('POST', '/auth/change-password', {'password': 'incorrect-audit-password', 'new_password': new_password}, expected=401)
    first.request('GET', '/auth/session'); second.request('GET', '/auth/session')
    first.request('POST', '/auth/change-password', {'password': password, 'new_password': new_password}, expected=204)
    first.request('GET', '/auth/session', expected=401); second.request('GET', '/auth/session', expected=401)
    second.request('POST', '/auth/sign-in', {'email': 'audit@example.test', 'password': password}, expected=401)
    sign_in(first, new_password); sign_in(second, new_password)
    print('PASS: password change rejects wrong password, accepts the new one, rejects the old one and invalidates all prior user sessions.', flush=True)
    enrollment = first.request('POST', '/auth/mfa/enroll', {'password': new_password})
    step = int(time.time())//30; accepted = {otp(enrollment['secret'], step+i) for i in [-1,0,1]}
    bad = next(f'{i:06d}' for i in range(1000000) if f'{i:06d}' not in accepted)
    first.request('POST', '/auth/mfa/confirm', {'code': bad}, expected=401)
    codes = first.request('POST', '/auth/mfa/confirm', {'code': otp(enrollment['secret'], int(time.time())//30)})['recovery_codes']
    assert len(codes)==8 and first.request('GET', '/auth/security')['two_factor_enabled']
    second.request('GET', '/auth/session', expected=401)
    challenge = sign_in(third, new_password); assert challenge['two_factor_required']
    third.request('POST', '/auth/sign-in/verify', {'code': codes[0]})
    third.request('GET', '/auth/session')
    first.request('POST', '/auth/reauthenticate', {'password': new_password, 'code': codes[0]}, expected=401)
    first.request('POST', '/auth/mfa/disable', {'password': new_password, 'code': codes[1]}, expected=204)
    status = first.request('GET', '/auth/security'); assert not status['two_factor_enabled'] and status['recovery_codes_remaining']==0
    third.request('GET', '/auth/session', expected=401)
    print('PASS: MFA setup/invalid code/enable, recovery-code login, one-use recovery protection and MFA disable work; current session retained and other sessions invalidated.', flush=True)
    first.request('POST', '/auth/sign-out', {}, expected=204)
    first.request('GET', '/auth/session', expected=401); unrelated.request('GET', '/auth/session')
    print('PASS: sign out invalidates its session and preserves another user.', flush=True)


live.scenario = scenario
if __name__ == '__main__':
    live.main()
