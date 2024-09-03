# Roadmap

- [x] implement dynamic tipping
- [x] see if can rewrite this to a single service without a http server, just a listener solely in rust
- [x] see if can trim the amount of token since every buy ends up being the same amount for some reason (in shitter units)
- [x] analyze the competitors
- [ ] find a suitable exit strategy, possibly dev sells, but can use the bulx or gmgn telegram bot to sell and snipe through grabbing its private key
- [x] consider sending the bundles through to all the jito block engine services, maybe the rejects are since the other validators receive the bundles first?
- [x] resolve the data issue, where I know about the pump listing from pump
      portal, sometimes way before and set a random deadline of 30 slots, it can be
      before the creation slot that is in the future as of receiving the data

* generally, the sniping is already quite fast, worst is 7-10 slots but average over 10 snipes was 2.3 slots once, pretty good can't lie
* i am faster than getgud and some of the other super-earning bots

--

* update: 0.54 slot avg over 100+ sniped tokens after using shredstream and fixing math
