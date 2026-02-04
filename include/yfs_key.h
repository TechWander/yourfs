#ifndef YFS_KEY_H
#define YFS_KEY_H

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <ctype.h>

char *keys[] =  {"FADEP8",
                 "XTPV63",
		 "23AGFQ",
		 "CA89CS",
		 "CDS87L",
		 "CD34VC",
		 "2APGFQ",
		 "XS420X",
		 "1409XT",
		 "SD76RD"};

char *dates[] = {"2027-01-01 00:00:00",
                 "2028-01-01 00:00:00",
                 "2029-01-01 00:00:00",
		 "2030-01-01 00:00:00",
		 "2031-01-01 00:00:00",
		 "2032-01-01 00:00:00",
		 "2033-01-01 00:00:00",
		 "2034-01-01 00:00:00",
		 "2035-01-01 00:00:00",
		 "2099-01-01 00:00:00"};


time_t string_to_time(const char *date_str, const char *format) {
    if (date_str == NULL || format == NULL) {
        return -1; // 输入参数无效
    }

    struct tm tm = {0}; // 创建一个struct tm变量

    // 使用sscanf根据格式解析字符串
    int parsed_values = sscanf(date_str, format,
                               &tm.tm_year, &tm.tm_mon, &tm.tm_mday,
                               &tm.tm_hour, &tm.tm_min, &tm.tm_sec);

    // 检查是否所有值都已解析
    int expected_values = 6; // 年、月、日、时、分、秒
    if (parsed_values != expected_values) {
        return -1; // 解析失败
    }

    // 调整tm结构体的字段
    tm.tm_year -= 1900; // 年份从1900开始
    if (tm.tm_mon > 11) { // 月份从0开始，且需要检查有效性
        return -1; // 解析失败
    }
    tm.tm_mon -= 1;

    // 将struct tm转换为time_t
    time_t time_value = mktime(&tm);
    if (time_value == -1) {
        return -1; // 时间转换失败
    }

    return time_value; // 返回转换后的时间值
}

//判断key的日期，是否在当前时间内
bool key_is_good(char *key_value)
{
  int i=0,cmp=0,count=-1;
  
  // 第几个符合
  for(i=0;i<10;i++)
  {
    cmp=strcmp(key_value,keys[i]);
    if(cmp==0)
    {
      count = i;
      break;
    }
  }
  
  // 若都不符合，count=-1
  if(count<0)
  {
    return false;
  }

  // 若符合，进行时间比较
  time_t now,key_date;
  time(&now);
  key_date = string_to_time(dates[count],"%d-%d-%d %d:%d:%d");
  if(now > key_date)
  {
    return false;
  }
  else
  {
    return true;
  }
}

  

#endif
