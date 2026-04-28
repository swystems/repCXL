#!/bin/bash

touch /dev/shm/repCXLnode0
touch /dev/shm/repCXLnode1
touch /dev/shm/repCXLlog

truncate -s 10M /dev/shm/repCXLnode0
truncate -s 10M /dev/shm/repCXLnode1
truncate -s 10M /dev/shm/repCXLlog