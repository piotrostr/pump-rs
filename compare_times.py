contents = """
1724845908933 3SuEgwWeUPs6mSW4uZjjzNMstnddtP8Zck2PavJ3b18xqh1Y6Cu5YLtcR7d1Xozy1edbSuU1Q4KeJPVZQaipoBC
1724845913963 4SQKD4YiQwtyNLsHHdG7SGznKd91SjhQprd8LeUU9HUefuhzQJPh1ibqHQtJsByKafyJ5thHa6qdiNJ9Qxo59LSz
1724845933275 5VTiKFz8PfAmoCkhuAV3VgQn3EhWuoKvxubRK9aSmjKvQwyWsLLXp7Er8StBzeKcHzkrvEr9ucAGoEcFLRTUmBwN
1724845949278 gG1s4r4MQmyhmWa5eifyPg9ZBMvu4bYLb1DNDBeZshsZrWFCc6mEReGA2W57UhyFyUqjN4caFy2HfCVNPbLMtZG
1724845970659 36G1tXS6UE9aMBSgoWhX2X7tYzYg52uLUsEMiXVw6DNLmdFzp4foyXCzJxpbDQudSJkx7BQBRAq7NH9Qz52XFXtM
1724845972815 4XKQJ41NmqkrWuFpWfjjssdxxCi9yawaNVWiKuzggzJREYS9w63XWtjAyruS3QL2zA64murP5VgaJwGkfT39G2B2
1724845978849 2VwjwpLfJk5esQRzdMKTG3gaEYdzKmJQKkFkX34xLN3EkWg9tPoq2RChXPxpKKwU6exgy4xkYn6g3JAwEH8xnHsY
1724845988053 2qZzcno5qTAYk76TB5CaogzsmUKCNN4L2FK2p6ZfumFts1AyLHRFR6jyVQzd5imwjsrY8eCTy3pUy4NwZdS8U6qq
--
1724845949423 gG1s4r4MQmyhmWa5eifyPg9ZBMvu4bYLb1DNDBeZshsZrWFCc6mEReGA2W57UhyFyUqjN4caFy2HfCVNPbLMtZG
1724845970845 36G1tXS6UE9aMBSgoWhX2X7tYzYg52uLUsEMiXVw6DNLmdFzp4foyXCzJxpbDQudSJkx7BQBRAq7NH9Qz52XFXtM
1724845973033 4XKQJ41NmqkrWuFpWfjjssdxxCi9yawaNVWiKuzggzJREYS9w63XWtjAyruS3QL2zA64murP5VgaJwGkfT39G2B2
1724845979069 2VwjwpLfJk5esQRzdMKTG3gaEYdzKmJQKkFkX34xLN3EkWg9tPoq2RChXPxpKKwU6exgy4xkYn6g3JAwEH8xnHsY
1724845988368 2qZzcno5qTAYk76TB5CaogzsmUKCNN4L2FK2p6ZfumFts1AyLHRFR6jyVQzd5imwjsrY8eCTy3pUy4NwZdS8U6qq
1724846003918 w7nFoXqvco87NWKt48oKPBeYkJu51Bu3R4tvSQuCJJ6aKhyZQJsfg5JgLpZUh9TDWqvjDV49kQkBGGZAiDfkMNb
1724846009062 3hmL6N6t7BCoBYZ1Nh4PDNKM1XZd7ZiEdxbf5hN2eJSLHCuuizuiS27V4bnc1LioriGqfK4HRqyiyGsJRUt2ehqL
1724846025851 5C2BCfx4XpQmw3SHohGRUbkDJ8CGBDMJw7nQSor1hkM3Z5VrbEhtK9gbEudgVRMx55Y6rJ4Fazbi2qXibPYZCAAZ
"""

if __name__ == "__main__":
    shreds, pp = contents.split("--")
    shreds = shreds.split("\n")
    pp = pp.split("\n")

    shreds = [s for s in shreds if s]
    pp = [p for p in pp if p]

    shred_times = {s.split(" ")[1]: s.split(" ")[0] for s in shreds}
    pp_times = {p.split(" ")[1]: p.split(" ")[0] for p in pp}

    for sig, timestamp in shred_times.items():
        if sig in pp_times:
            diff = int(timestamp) - int(pp_times[sig])
            print(f"{sig} diff: {diff}")
        else:
            print(f"Missing {sig} in pp_times")

    for sig, timestamp in pp_times.items():
        if sig not in shred_times:
            print(f"Missing {sig} in shred_times")
