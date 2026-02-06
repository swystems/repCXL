#!/bin/bash

touch /dev/shm/repCXL_test1
touch /dev/shm/repCXL_test2
touch /dev/shm/repCXL_test3

truncate -s 1M /dev/shm/repCXL_test1
truncate -s 1M /dev/shm/repCXL_test2
truncate -s 1M /dev/shm/repCXL_test3