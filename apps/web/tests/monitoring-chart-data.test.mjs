import { test } from 'node:test'
import assert from 'node:assert/strict'
import { insertChartGaps, retainedChartData } from '../src/lib/monitoring-chart-data.ts'
const base = Date.parse('2026-11-28T00:00:00Z')
const point = (id, second, data, extra = {}) => ({ id, collected_at: new Date(base + second * 1000).toISOString(), container_id: 'a', deployment_id: 'd1', image_id: 'image', error: false, marker: false, data, ...extra })
const history = samples => ({ samples, sample_seconds: 5, downsample_seconds: 60, downsample_after_minutes: 60, retention_days: 7, truncated: false, scope: 'test', gaps_note: '' })
test('retains null readings and zero values without fabricating table samples', () => {
 const result = retainedChartData(history([point(1,0,{cpu:0,memory_used:0,memory_total:100}), point(2,5,{}), point(3,10,{cpu:25})]), 'percent',base+10000)
 assert.deepEqual(result.rows.map(row=>[row.a,row.b]),[[0,0],[null,null],[25,null]])
 assert.equal(result.rows.length,3)
})
test('inserts null chart-only rows at long collection gaps and container changes', () => {
 const result=retainedChartData(history([point(1,0,{cpu:10}),point(2,30,{cpu:20}),point(3,35,{cpu:30},{container_id:'b',deployment_id:'d2',marker:true})]),'percent',base+35000)
 assert.equal(result.rows.length,3)
 assert.equal(result.chartRows.length,5)
 assert.deepEqual(result.chartRows.filter(row=>row.id.startsWith('gap-')).map(row=>[row.a,row.b]),[[null,null],[null,null]])
 assert.equal(result.markers[0].at,base+35000)
 assert.match(result.markers[0].label,/d2/)
})
test('does not bridge errored samples or calculate rate across restart/counter reset', () => {
 const result=retainedChartData(history([point(1,0,{network_rx_bytes:100}),point(2,5,{network_rx_bytes:200}),point(3,10,{network_rx_bytes:999},{error:true}),point(4,15,{network_rx_bytes:300}),point(5,20,{network_rx_bytes:10},{container_id:'b',marker:true}),point(6,25,{network_rx_bytes:5},{container_id:'b'})]),'network',base+25000)
 assert.deepEqual(result.rows.map(row=>row.a),[null,20,null,null,null,null])
})
test('honors downsampled cadence instead of marking valid older samples as gaps', () => {
 const result=retainedChartData(history([point(1,0,{cpu:10}),point(2,60,{cpu:20})]),'percent',base+7200000)
 assert.equal(result.chartRows.length,2)
})
test('host gaps preserve input and use timestamps rather than equally spaced categories',()=>{
 const rows=[{id:'1',at:0,cpu:1},{id:'2',at:20000,cpu:2}]
 const chart=insertChartGaps(rows,['cpu'],(a,b)=>b.at-a.at>15000)
 assert.equal(chart[1].at,10000);assert.equal(chart[1].cpu,null);assert.equal(rows.length,2)
})
