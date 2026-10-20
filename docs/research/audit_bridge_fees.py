"""Offline replay of catalog fee rates (requires Python solders).
Usage: python audit_bridge_fees.py /path/to/POOL-snapshot.json
Reports the initial tick/bin fee at snapshot Clock, not whole-order average.
"""
import base64
import json
import sys
from pathlib import Path
from solders.pubkey import Pubkey
CL = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"
DL = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
def pk(b): return str(Pubkey.from_bytes(b))
def u(d,o,n=4):return int.from_bytes(d[o:o+n],'little')
def i(d,o,n=4):return int.from_bytes(d[o:o+n],'little',signed=True)
def ceil(a,b):return (a+b-1)//b
def token_fee(a,epoch):
 d=base64.b64decode(a['data'][0]);assert a['owner'] in ['TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA','TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb']
 result={'basis_points':0,'maximum_fee_raw':0,'next_basis_points':0}
 if a['owner'].startswith('Tokenkeg'):return result
 o=166
 while o+4<=len(d):
  tag,n=u(d,o,2),u(d,o+2,2)
  if tag==0:break
  b=d[o+4:o+4+n];assert len(b)==n
  if tag==1:
   assert len(b)==108
   pos=90 if epoch>=u(b,90,8) else 72
   result={'basis_points':u(b,pos+16,2),'maximum_fee_raw':u(b,pos+8,8),'next_basis_points':u(b,106,2),'next_epoch':u(b,90,8)}
  o+=4+n
 return result

def pool_fee(s,pool):
 accounts=s['accounts'];a=accounts[pool];d=base64.b64decode(a['data'][0]);clock=base64.b64decode(accounts['SysvarC1ock11111111111111111111111111111111']['data'][0]);t=i(clock,32,8)
 def data(key):return base64.b64decode(accounts[key]['data'][0])
 owner=a['owner'];dynamic=False
 if owner==CL:
  c=data(pk(d[9:41]));base=u(c,47)/100;rate=base;z=d[1096:1176];dynamic=any(z)
  if dynamic:
   spacing=u(d,235,2);group=i(d,269)//spacing;ref=i(z,14);vr=u(z,18)
   if t-u(z,26,8)>=u(z,0,2):ref=group;vr=u(z,22)*u(z,4,2)//10000 if t-u(z,26,8)<u(z,2,2) else 0
   va=min(vr+abs(ref-group)*10000,u(z,10));rate=min(u(c,47)+ceil(u(z,6)*(va*spacing)**2,10**13),100000)/100
 elif owner==DL:
  step=u(d,80,2);base_raw=u(d,8,2)*step*10*10**d[34];base=base_raw/100000;dynamic=u(d,16)!=0
  ref=i(d,48);vr=u(d,44);active=i(d,76);elapsed=t-i(d,56,8)
  if elapsed>=u(d,10,2):ref=active;vr=u(d,40)*u(d,14,2)//10000 if elapsed<u(d,12,2) else 0
  va=min(vr+abs(ref-active)*10000,u(d,20));rate=min(base_raw+ceil(u(d,16)*(va*step)**2,10**11),100000000)/100000
 elif owner.startswith('whir'):
  base=u(d,45,2)/100;rate=base;dynamic=u(d,43,2)!=u(d,41,2)
  if dynamic:
   oracle=str(Pubkey.find_program_address([b'oracle',bytes(Pubkey.from_string(pool))],Pubkey.from_string(owner))[0]);z=data(oracle)
   group=i(d,81)//u(z,62,2);ref=i(z,102);vr=u(z,98);elapsed=t-max(u(z,82,8),u(z,90,8));assert elapsed>=0
   if t-u(z,82,8)>3600:ref=group;vr=0
   elif elapsed>=u(z,48,2):ref=group;vr=u(z,106)*u(z,52,2)//10000 if elapsed<u(z,50,2) else 0
   va=min(vr+abs(ref-group)*10000,u(z,58));rate=min(u(d,45,2)+ceil(u(z,54)*(va*u(z,62,2))**2,10**13),100000)/100
 else:raise ValueError('Unsupported fee audit owner '+owner)
 return {'base_fee_bps':base,'snapshot_start_fee_bps':rate,'dynamic':dynamic,'scope':'Initial active bin/tick rate at snapshot Clock; may change during swap; protocol share not added twice.','slot':s['slot'],'unix_timestamp':t}

if __name__ == "__main__":
    snapshot = json.loads(Path(sys.argv[1]).read_text())
    clock = base64.b64decode(snapshot["accounts"]["SysvarC1ock11111111111111111111111111111111"]["data"][0])
    epoch = u(clock, 16, 8)
    for hint in snapshot["hints"]:
        result = pool_fee(snapshot, hint["pool"])
        result["stock_transfer_fee"] = token_fee(snapshot["accounts"][hint["output_mint"]], epoch)
        result["quote_transfer_fee"] = token_fee(snapshot["accounts"][hint["input_mint"]], epoch)
        print(json.dumps({"pool": hint["pool"], "fees": result}, indent=2))
