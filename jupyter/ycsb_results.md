# local tests

> # clients: 4
> # mem nodes: 3
> workload: workloada_64_large

	
		        | tput	| read avg\99.99 | write avg\99.99 | dirty-reads | conflicts | sync failures | 
----------------------------------------------------------------------------------------------------
async best eff. | 822k	| 855ns\7.5us	 | 1.3us\10.7us	   | 0.035%	     | x         | x             |
swarm (paper)   | 264k  | 2.7us\2.8us    | 3.1us\?         | 0           | ?         | x             |
monster-1us	    | 473k  | 852ns\9.1us	 | 3.1us\17us	   | 0.019% 	 | 0	     | 0.33%	     |	
monster-2us	    | 293k  | 852ns\15us	 | 5.7us\15us      | 0.01%       | 0         | 0.47%         |
monster-4us	    | 161k  | 854ns\9.1us    | 11.2us\46us     | 0.007%      | 0         | 0.6%          |
monster-10us	| 66k   | 865ns\10us     | 28us\109us      | 0.001%      | 0         | 0.007%        |

> # clients: 4 
> # mem nodes: 3
> workload: workloadb_64_large

Note: optimized readends shows always reads 2 nodes no matter the # of mem nodes.

		        | tput	| read avg\99.99 | write avg\99.99 | dirty-reads | conflicts | sync failures | 
----------------------------------------------------------------------------------------------------
async best eff. | 1.15M | 730ns\7.5us	 | 1.3us\19us	   | 0.003%	     | x         | x             |
swarm (paper)   | 389k  | 2.4us\2.8us    | 3.1us\?         | 0           | ?         | x             |
monster-1us	    | 1.04M | 730ns\9.9us	 | 3.35us\79us	   | 0.003% 	 | 0	     | 0.13%	     |	
monster-2us	    | 923k  | 730ns\9.4us	 | 5.9us\161us     | 0.001%      | 0         | 0.14%         |
monster-4us	    | 740k  | 730ns\9.8us    | 11us\71us       | 0.001%      | 0         | 0.11%         |
monster-10us	| 470k  | 730ns\9.5us    | 26us\640us      | 0.0009%     | 0         | 0.002%        |

> # clients: 4 
> # mem nodes: 3
> workload: workloada_64_contention

		         | tput	| read avg | write avg | dirty-reads | conflicts | sync failures | 
----------------------------------------------------------------------------------------------------
async rr=0       | 820k  | 890ns   | 1.5us     | 1.9%	     | x         | x             |
async rr=1       | 790k  | 910ns   | 1.45us	   | 0.34%	     | x         | x             |
async rr=10      | 809k  | 920ns   | 1.32us	   | 0.19%	     | x         | x             |
monster-1us rr=0 | 444k  | 850ns   | 3.40us	   | 0.7%   	 | 0	     | ?    	     |	
monster-1us rr=1 | 410K  | 900ns   | 3.60us	   | 0.08%   	 | 10	     | ?    	     |	
monster-1us rr=10| 410k  | 900ns   | 3.62us	   | 0.06%   	 | 0	     | ?    	     |	

