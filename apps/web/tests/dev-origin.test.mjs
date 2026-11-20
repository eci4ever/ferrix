import {test} from 'node:test'
import assert from 'node:assert/strict'
import {localOriginRedirect} from '../src/lib/dev-origin.ts'
const canonical='http://127.0.0.1:5173'
test('local document navigation preserves path and query on the canonical auth origin',()=>{
 assert.equal(localOriginRedirect(canonical,'localhost:5173','GET','text/html','/signup?from=home'),canonical+'/signup?from=home')
 assert.equal(localOriginRedirect(canonical,'[::1]:5173','HEAD','text/html','/login'),canonical+'/login')
 assert.equal(localOriginRedirect('http://localhost:5173','127.0.0.1:5173','GET','text/html','/login'),'http://localhost:5173/login')
})
test('does not redirect API requests, other users ports, production origins or untrusted hosts',()=>{
 for (const [origin,host,method,accept,target] of [
  [canonical,'127.0.0.1:5173','GET','text/html','/login'],
  [canonical,'localhost:5174','GET','text/html','/login'],
  [canonical,'evil.example:5173','GET','text/html','/login'],
  ['https://deploy.example.com','localhost:5173','GET','text/html','/login'],
  [canonical,'localhost:5173','POST','text/html','/api/v1/auth/setup'],
  [canonical,'localhost:5173','GET','application/json','/api/v1/auth/session'],
  [canonical,'localhost:5173','GET','text/html','/api/v1/auth/session'],
  [canonical,'localhost:5173','GET','text/html','https://evil.example/'],
  ['invalid','localhost:5173','GET','text/html','/login'],
 ]) assert.equal(localOriginRedirect(origin,host,method,accept,target),null)
})
