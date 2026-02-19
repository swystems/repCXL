# notes for benchmarking

## yscb

Build yscb from source for python 3 support (default 0.17 bin uses Python 2)


On Ubuntu 24.04, kernel 2.17.4
```sh
sudo apt update
sudo apt install default-jdk maven
# clone ycsb repo then build yscb with Redis binding
git clone https://github.com/brianfrankcooper/YCSB.git
cd YCSB
mvn -q -B -pl site.ycsb:redis-binding -am clean package 
```

install redis 
    
    sudo apt install redis # by default runs as systemd service

run yscb bench

    bin/ycsb load redis -P workloads/workloada -p "redis.host=127.0.0.1" -p "redis.port=6379
    bin/ycsb run redis -P workloads/workloada -p "redis.host=127.0.0.1" -p "redis.port=6379