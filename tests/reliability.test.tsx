import React from 'react';
import { act, create } from 'react-test-renderer';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { ProgressPanel } from '../src/components/BackupView';

vi.mock('../src/api', async (original) => ({ ...(await original()), onBackupProgress: vi.fn(async () => vi.fn()), dockerHealth: vi.fn(), listConnections: vi.fn(), listDestinations: vi.fn(), listRuns: vi.fn(), listSchedules: vi.fn(), runScheduleNow: vi.fn(), testConnection: vi.fn() }));
const mounted: any[] = [];
function render(element: React.ReactElement) { let view: any; act(() => { view = create(element); }); mounted.push(view); return view; }
afterEach(() => { act(() => { mounted.splice(0).forEach(v => v.unmount()); }); vi.clearAllMocks(); vi.unstubAllGlobals(); });
function stage(view: any, name: string) { return view.root.findAllByType('li').find((li: any) => li.children.includes(name)); }
import { useBackup, useAsync } from '../src/hooks';
import * as api from '../src/api';
function deferred() { let resolve: any, reject: any; const promise = new Promise<any>((a,b) => { resolve=a; reject=b; }); return {promise,resolve,reject}; }
async function flush() { await act(async () => { await Promise.resolve(); }); }

it('shares running state and prevents overlap across consumers after page unmount', async () => {
  let first: any, second: any;
  function First() { first = useBackup('first'); return null; }
  function Second() { second = useBackup('second'); return null; }
  const view = render(<First />);
  const pending = deferred();
  let execution: any;
  act(() => { execution = first.run(() => pending.promise); });
  await flush();
  act(() => view.update(<Second />));
  expect(second.running).toBe(true);
  const overlap = vi.fn();
  await act(async () => { await second.run(overlap); });
  expect(overlap).not.toHaveBeenCalled();
  await act(async () => { pending.resolve({state:'Completed',destinations:[]}); await execution; });
});

it('waits for one listener and releases it after consumer unmount, normalizing unknown rejection', async () => {
  const registration = deferred();
  const unlisten = vi.fn();
  vi.mocked(api.onBackupProgress).mockReturnValueOnce(registration.promise);
  let backup: any;
  function Consumer() { backup = useBackup('cleanup'); return null; }
  const view = render(<Consumer />);
  const command = vi.fn(async () => { throw 'network lost'; });
  let execution: any;
  act(() => { execution = backup.run(command); });
  expect(command).not.toHaveBeenCalled();
  act(() => view.unmount());
  await act(async () => { registration.resolve(unlisten); await execution; });
  const restored = render(<Consumer />);
  expect(backup.error.message).toBe('network lost');
  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(api.onBackupProgress).toHaveBeenCalledTimes(1);
  expect(restored).toBeTruthy();
});

import App from '../src/App';
it('keeps operation details accessible above pages after navigation', async () => {
  vi.stubGlobal('window', new EventTarget());
  vi.mocked(api.dockerHealth).mockResolvedValue({installed:true,running:true,version:'test'});
  vi.mocked(api.listConnections).mockResolvedValue([]);
  vi.mocked(api.listDestinations).mockResolvedValue([]);
  vi.mocked(api.listRuns).mockResolvedValue([]);
  let backup: any;
  function Initiator() { backup = useBackup('Saved database 7'); return null; }
  const initiator = render(<Initiator />);
  const pending = deferred();
  let execution: any;
  act(() => { execution = backup.run(() => pending.promise); });
  await flush();
  act(() => initiator.unmount());
  const view = render(<App />);
  await flush();
  act(() => view.root.findAllByType('button').find((b:any) => b.children.includes('Settings')).props.onClick());
  expect(JSON.stringify(view.toJSON())).toContain('Backup operation');
  const details = view.root.findAllByType('details')[0];
  expect(details).toBeTruthy();
  await act(async () => { pending.resolve({state:'PartialSuccess',destinations:[],verification:'Passed'}); await execution; });
  expect(JSON.stringify(view.toJSON())).toContain('PartialSuccess');
});

it('ignores superseded async data and errors, including after unmount', async () => {
  const old = deferred(), latest = deferred();
  let data: any;
  const loader = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
  function Consumer() { data = useAsync(loader, []); return null; }
  const view = render(<Consumer />);
  act(() => { void data.reload(); });
  await act(async () => { latest.resolve('fresh'); });
  expect(data.data).toBe('fresh');
  await act(async () => { old.reject('outdated failure'); });
  expect(data.error).toBeNull();
  expect(data.data).toBe('fresh');
  const abandoned = deferred();
  loader.mockReturnValueOnce(abandoned.promise);
  act(() => { void data.reload(); });
  act(() => view.unmount());
  await act(async () => { abandoned.resolve('abandoned'); });
  expect(data.data).toBe('fresh');
});

it('shows Docker check failure, retries manually and on focus without overlap', async () => {
  const target = new EventTarget();
  vi.stubGlobal('window', target);
  vi.mocked(api.dockerHealth).mockRejectedValueOnce(new Error('check failed'));
  const view = render(<App />);
  await flush();
  expect(JSON.stringify(view.toJSON())).toContain('Docker check failed');
  expect(JSON.stringify(view.toJSON())).not.toContain('Docker not installed');
  const pending = deferred();
  vi.mocked(api.dockerHealth).mockReturnValueOnce(pending.promise);
  const retry = view.root.findAllByType('button').find((b:any) => b.children.includes('Recheck Docker'));
  act(() => { retry.props.onClick(); target.dispatchEvent(new Event('focus')); });
  expect(api.dockerHealth).toHaveBeenCalledTimes(2);
  await act(async () => { pending.resolve({installed:false,running:false,version:null}); });
  expect(JSON.stringify(view.toJSON())).toContain('Docker not installed');
  vi.mocked(api.dockerHealth).mockResolvedValueOnce({installed:true,running:true,version:'mock'});
  act(() => { target.dispatchEvent(new Event('focus')); });
  await flush();
  expect(api.dockerHealth).toHaveBeenCalledTimes(3);
  expect(JSON.stringify(view.toJSON())).toContain('Docker mock');
  act(() => view.unmount());
  target.dispatchEvent(new Event('focus'));
  expect(api.dockerHealth).toHaveBeenCalledTimes(3);
});

import Dashboard from '../src/pages/Dashboard';
it('shows Dashboard loading rather than zero and offers Retry after data errors', async () => {
  const connections = deferred();
  vi.mocked(api.listConnections).mockReturnValueOnce(connections.promise);
  const view = render(<Dashboard />);
  expect(JSON.stringify(view.toJSON())).toContain('Loading…');
  expect(JSON.stringify(view.toJSON())).not.toContain('0 connected');
  await act(async () => { connections.reject('unavailable'); });
  expect(JSON.stringify(view.toJSON())).toContain('unavailable');
  vi.mocked(api.listConnections).mockResolvedValueOnce([{id:7}] as any);
  const retry = view.root.findAllByType('button').find((b:any) => b.children.includes('Retry'));
  await act(async () => { await retry.props.onClick(); });
  expect(JSON.stringify(view.toJSON())).toContain('1 saved');
});

it('separates the latest attempt from successful backups and qualifies the limited history', async () => {
  vi.mocked(api.listRuns).mockResolvedValueOnce([
    {id:1,status:'PartialSuccess',started_at:'2026-10-13'},
    {id:2,status:'Completed',started_at:'2026-10-12'},
  ] as any);
  const view = render(<Dashboard />);
  await flush();
  const stats = view.root.findAll((n:any) => n.props.className === 'card stat');
  expect(JSON.stringify(stats.map((n:any) => n.children.map((c:any) => c.children)))).toContain('Latest attempt');
  const successful = stats.find((n:any) => JSON.stringify(n.children.map((c:any) => c.children)).includes('Last successful'));
  expect(JSON.stringify(successful.children.map((c:any) => c.children))).toContain(api.fmtDate('2026-10-12'));
  expect(JSON.stringify(view.toJSON())).toContain('older successful backups may not be shown');
});

import Schedules from '../src/pages/Schedules';
it('routes schedule Run now through the same operation owner and handles rejection', async () => {
  vi.mocked(api.listSchedules).mockResolvedValueOnce([{policy:{id:9,name:'Daily',enabled:true},connection_name:'Test',schedule:null,task:{},destination_ids:[]}] as any);
  const pending = deferred();
  vi.mocked(api.runScheduleNow).mockReturnValueOnce(pending.promise);
  let backup: any;
  function Observer() { backup = useBackup('Schedule 9: Daily'); return null; }
  const view = render(<><Schedules /><Observer /></>);
  await flush();
  const button = view.root.findAllByType('button').find((b:any) => b.children.includes('Run now'));
  act(() => { button.props.onClick(); });
  await flush();
  expect(backup.running).toBe(true);
  expect(button.props.disabled).toBe(true);
  await act(async () => { pending.reject('schedule failed'); });
  expect(backup.error.message).toBe('schedule failed');
  expect(api.onBackupProgress).toHaveBeenCalledTimes(1);
});

it('ignores late events from the previous operation and records explicit completion', async () => {
  const callbacks: any[] = [];
  vi.mocked(api.onBackupProgress).mockImplementation(async (cb:any) => { callbacks.push(cb); return vi.fn(); });
  let backup: any;
  function Consumer() { backup = useBackup('event isolation'); return null; }
  render(<Consumer />);
  await act(async () => { await backup.run(async () => ({state:'Completed',destinations:[]})); });
  expect(backup.currentStage).toBe('Completed');
  const pending = deferred();
  let execution: any;
  act(() => { execution = backup.run(() => pending.promise); });
  await flush();
  act(() => { callbacks[0]({state:'Completed',message:'old'}); callbacks[1]({state:'Dumping',message:'new'}); });
  expect(backup.events.map((e:any) => e.message)).toEqual(['new']);
  await act(async () => { pending.resolve({state:'PartialSuccess',destinations:[]}); await execution; });
  expect(backup.currentStage).toBe('PartialSuccess');
});

import DevBackup from '../src/pages/DevBackup';
import Databases from '../src/pages/Databases';
const savedConnection = {id:7,config:{name:'Saved test database',username:'tester',host:'mock',port:5432,database:'test'}};
it.each([['plain failure','plain failure'], [new Error('native failure'),'native failure'], [null,'An unexpected error occurred.']])('normalizes saved ConnectionCard Test rejection: %s', async (error, message) => {
  vi.mocked(api.listConnections).mockResolvedValueOnce([savedConnection] as any);
  vi.mocked(api.testConnection).mockRejectedValueOnce(error);
  const view = render(<Databases />);
  await flush();
  const button = view.root.findAllByType('button').find((b:any) => b.children.includes('Test'));
  await act(async () => { await button.props.onClick(); });
  expect(api.testConnection).toHaveBeenCalledWith(7);
  expect(JSON.stringify(view.toJSON())).toContain(message);
  expect(button.props.disabled).toBe(false);
});
it.each(['resolve', 'reject'])('ignores superseded saved ConnectionCard Test %s', async (outcome) => {
  const old = deferred(), latest = deferred();
  vi.mocked(api.listConnections).mockResolvedValueOnce([savedConnection] as any);
  vi.mocked(api.testConnection).mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
  const view = render(<Databases />);
  await flush();
  const button = view.root.findAllByType('button').find((b:any) => b.children.includes('Test'));
  let first: any, second: any;
  // Invoke the retained handler to model requests superseded before a UI update.
  act(() => { first = button.props.onClick(); second = button.props.onClick(); });
  await act(async () => {
    if (outcome === 'resolve') old.resolve({postgres_major:16});
    else old.reject('stale failure');
    await first;
  });
  expect(JSON.stringify(view.toJSON())).toContain('Testing…');
  expect(JSON.stringify(view.toJSON())).not.toContain('stale failure');
  expect(JSON.stringify(view.toJSON())).not.toContain('PostgreSQL 16');
  expect(button.props.disabled).toBe(true);
  await act(async () => { latest.resolve({postgres_major:17}); await second; });
  expect(JSON.stringify(view.toJSON())).toContain('Connected · PostgreSQL 17');
  expect(button.props.disabled).toBe(false);
});
it.each(['resolve', 'reject'])('settles saved ConnectionCard Test safely after unmount: %s', async (outcome) => {
  const pending = deferred();
  vi.mocked(api.listConnections).mockResolvedValue([savedConnection] as any);
  vi.mocked(api.testConnection).mockReturnValueOnce(pending.promise);
  const view = render(<Databases />);
  await flush();
  const button = view.root.findAllByType('button').find((b:any) => b.children.includes('Test'));
  let execution: any;
  act(() => { execution = button.props.onClick(); });
  act(() => view.unmount());
  await act(async () => {
    if (outcome === 'resolve') pending.resolve({postgres_major:16});
    else pending.reject(null);
    await execution;
  });
  const reopened = render(<Databases />);
  await flush();
  expect(JSON.stringify(reopened.toJSON())).not.toContain('Connected');
  expect(JSON.stringify(reopened.toJSON())).not.toContain('An unexpected error occurred.');
  expect(reopened.root.findAllByType('button').find((b:any) => b.children.includes('Test')).props.disabled).toBe(false);
});
it.each(['quick', 'saved'])('ignores connection-test responses after input changes in %s form', async (kind) => {
  const pending = deferred();
  vi.spyOn(api, 'testConnectionUri').mockReturnValueOnce(pending.promise);
  const view = render(kind === 'quick' ? <DevBackup /> : <Databases />);
  await flush();
  if (kind === 'saved') act(() => view.root.findAllByType('button').find((b:any) => b.children.includes('+ Add Database')).props.onClick());
  const uri = view.root.findAllByType('input').find((n:any) => n.props.placeholder?.startsWith('postgresql:'));
  act(() => uri.props.onChange({target:{value:'old uri'}}));
  const button = view.root.findAllByType('button').find((b:any) => b.children.includes('Test Connection'));
  act(() => { button.props.onClick(); });
  act(() => uri.props.onChange({target:{value:'new uri'}}));
  await act(async () => { pending.resolve({postgres_major:17,postgres_text:'stale connection',engine_image:'mock'}); });
  expect(JSON.stringify(view.toJSON())).not.toContain('stale connection');
  expect(button.props.disabled).toBe(false);
});

import { ErrorPanel } from '../src/components/BackupView';
it.each([['plain failure','plain failure'], [new Error('native failure'),'native failure'], [null,'An unexpected error occurred.']])('renders unknown errors safely: %s', (error, message) => {
  const view = render(<ErrorPanel error={error} />);
  expect(JSON.stringify(view.toJSON())).toContain(message);
});

it('does not claim an empty retained history means no backups have ever run', async () => {
  vi.mocked(api.listRuns).mockResolvedValueOnce([]);
  const view = render(<Dashboard />);
  await flush();
  expect(JSON.stringify(view.toJSON())).toContain('No recent attempts');
  expect(JSON.stringify(view.toJSON())).not.toContain('No backups yet');
});

describe('progress', () => {
  it('keeps the current stage active until advance and never ticks Failed', () => {
    const view = render(<ProgressPanel events={[{state:'Connecting',message:'start'}]} running currentStage="Connecting" />);
    expect(stage(view, 'Connecting').props.className).toBe('active');
    act(() => view.update(<ProgressPanel events={[{state:'Connecting',message:'start'},{state:'Dumping',message:'dump'},{state:'Failed',message:'bad'}]} running={false} currentStage="Failed" />));
    expect(stage(view, 'Connecting').props.className).toBe('done');
    expect(stage(view, 'Dumping').props.className).not.toBe('done');
    expect(stage(view, 'Completed').props.className).not.toBe('done');
  });
});
