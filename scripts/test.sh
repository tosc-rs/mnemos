#!/usr/bin/env bash

source scripts/_util.sh

if confirm "test"; then 
echo "ok you confirmed"
else
echo "you didn't confirm"
fi
