import React, { useEffect, useRef } from 'react';
import { Linking } from 'react-native';
import { NavigationContainer, NavigationContainerRef } from '@react-navigation/native';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { enableScreens } from 'react-native-screens';

enableScreens();

import RegisterScreen from './screens/RegisterScreen';
import HomeScreen from './screens/HomeScreen';
import ProposalsScreen from './screens/ProposalsScreen';
import LawsScreen from './screens/LawsScreen';
import PetitionScreen from './screens/PetitionScreen';
import DelegateScreen from './screens/DelegateScreen';
import DelegateDetailScreen from './screens/DelegateDetailScreen';
import RegisterDelegateScreen from './screens/RegisterDelegateScreen';
import AuthScreen from './screens/AuthScreen';
import VoteScreen from './screens/VoteScreen';

export type RootStackParamList = {
  Main: undefined;
  Register: undefined;
  Auth: { deepLink?: string };
  DelegateDetail: { address: string };
  RegisterDelegate: undefined;
};

export type TabParamList = {
  Home: undefined;
  Proposals: undefined;
  Laws: undefined;
  Petitions: undefined;
  Delegate: undefined;
  Budget: undefined;
};

const Stack = createNativeStackNavigator<RootStackParamList>();
const Tab = createBottomTabNavigator<TabParamList>();

function MainTabs() {
  return (
    <Tab.Navigator
      screenOptions={{
        tabBarStyle: { backgroundColor: '#0f1117', borderTopColor: '#1f2937' },
        tabBarActiveTintColor: '#6C63FF',
        tabBarInactiveTintColor: '#6b7280',
        headerStyle: { backgroundColor: '#0f1117' },
        headerTintColor: '#ffffff',
      }}
    >
      <Tab.Screen name="Home" component={HomeScreen} options={{ title: 'Agora', headerShown: false }} />
      <Tab.Screen name="Proposals" component={ProposalsScreen} />
      <Tab.Screen name="Laws" component={LawsScreen} />
      <Tab.Screen name="Petitions" component={PetitionScreen} />
      <Tab.Screen name="Delegate" component={DelegateScreen} />
      <Tab.Screen name="Budget" component={VoteScreen} />
    </Tab.Navigator>
  );
}

export default function App() {
  const navRef = useRef<NavigationContainerRef<RootStackParamList>>(null);

  // Handle deep links: democracychain://auth?challenge=...&callback=...
  useEffect(() => {
    function handleDeepLink(url: string | null) {
      if (!url) return;
      try {
        const parsed = new URL(url);
        if (parsed.host === 'auth') {
          navRef.current?.navigate('Auth', { deepLink: url });
        }
      } catch { /* ignore malformed URLs */ }
    }

    Linking.getInitialURL().then(handleDeepLink);
    const sub = Linking.addEventListener('url', ({ url }) => handleDeepLink(url));
    return () => sub.remove();
  }, []);

  return (
    <SafeAreaProvider>
    <NavigationContainer ref={navRef}>
      <Stack.Navigator
        screenOptions={{
          headerStyle: { backgroundColor: '#0f1117' },
          headerTintColor: '#ffffff',
          contentStyle: { backgroundColor: '#0f1117' },
        }}
      >
        <Stack.Screen name="Main" component={MainTabs} options={{ headerShown: false }} />
        <Stack.Screen
          name="Register"
          component={RegisterScreen}
          options={{ title: 'Register as Citizen' }}
        />
        <Stack.Screen
          name="Auth"
          component={AuthScreen}
          options={{ title: 'Desktop Sign-In', presentation: 'modal' }}
        />
        <Stack.Screen
          name="DelegateDetail"
          component={DelegateDetailScreen}
          options={{ title: 'Delegate Profile' }}
        />
        <Stack.Screen
          name="RegisterDelegate"
          component={RegisterDelegateScreen}
          options={{ title: 'Become a Delegate' }}
        />
      </Stack.Navigator>
    </NavigationContainer>
    </SafeAreaProvider>
  );
}
