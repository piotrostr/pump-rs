#!/bin/bash

cargo run --features debug -- launch \
    --name "test" \
    --symbol "dupa" \
    --description "sraka" \
    --telegram "" \
    --twitter "" \
    --website "" \
    --image-path ./police-car-light.png \
    --dev-buy 10000000 \
    --snipe-buy 1000000
