import requests
import time

BASE_URL = "http://localhost:8080"

def send_command(command_payload):
    url = f"{BASE_URL}/command"
    try:
        requests.post(url, json=command_payload)
    except Exception as e:
        pass

print("Injecting transactions for Block 2")
send_command({"CreateAuction": {"auction_id": "MOTO1", "item_name": "Yamaha R1", "starting_price": 15000}})
send_command({"StartAuction": {"auction_id": "MOTO1"}})
send_command({"PlaceBid": {"auction_id": "MOTO1", "bid_amount": 16000, "bidder_id": "Alice"}})
print("Transactions in mempool. Use /mine")