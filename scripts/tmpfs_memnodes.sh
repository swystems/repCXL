#!/bin/bash

touch /dev/shm/repCXLnode0
touch /dev/shm/repCXLnode1

truncate -s 1M /dev/shm/repCXLnode0
truncate -s 1M /dev/shm/repCXLnode1